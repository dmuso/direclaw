use crate::config::{OrchestratorConfig, Settings, WorkflowConfig, WorkflowStepConfig};
use crate::queue::IncomingMessage;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("channel message `{message_id}` is missing `channelProfileId`")]
    MissingChannelProfileId { message_id: String },
    #[error("unknown channel profile `{channel_profile_id}`")]
    UnknownChannelProfileId { channel_profile_id: String },
    #[error("selector result is not valid json: {0}")]
    SelectorJson(String),
    #[error("selector validation failed: {0}")]
    SelectorValidation(String),
    #[error("unknown function id `{function_id}`")]
    UnknownFunction { function_id: String },
    #[error("workflow run state transition `{from}` -> `{to}` is invalid")]
    InvalidRunTransition { from: RunState, to: RunState },
    #[error("workflow result envelope parse failed: {0}")]
    WorkflowEnvelope(String),
    #[error("workflow review decision must be `approve` or `reject`, got `{0}`")]
    InvalidReviewDecision(String),
    #[error("workflow execution exceeded max total iterations ({max_total_iterations})")]
    MaxIterationsExceeded { max_total_iterations: u32 },
    #[error("workflow run timed out after {run_timeout_seconds}s")]
    RunTimeout { run_timeout_seconds: u64 },
    #[error("workflow step timed out after {step_timeout_seconds}s")]
    StepTimeout { step_timeout_seconds: u64 },
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json error at {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

fn io_error(path: &Path, source: std::io::Error) -> OrchestratorError {
    OrchestratorError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn json_error(path: &Path, source: serde_json::Error) -> OrchestratorError {
    OrchestratorError::Json {
        path: path.display().to_string(),
        source,
    }
}

pub fn resolve_orchestrator_id(
    settings: &Settings,
    inbound: &IncomingMessage,
) -> Result<String, OrchestratorError> {
    let channel_profile_id = inbound
        .channel_profile_id
        .as_ref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| OrchestratorError::MissingChannelProfileId {
            message_id: inbound.message_id.clone(),
        })?;

    let profile = settings
        .channel_profiles
        .get(channel_profile_id)
        .ok_or_else(|| OrchestratorError::UnknownChannelProfileId {
            channel_profile_id: channel_profile_id.to_string(),
        })?;
    Ok(profile.orchestrator_id.clone())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectorRequest {
    pub selector_id: String,
    pub channel_profile_id: String,
    pub message_id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    pub user_message: String,
    pub available_workflows: Vec<String>,
    pub default_workflow: String,
    #[serde(default)]
    pub available_functions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorStatus {
    Selected,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorAction {
    WorkflowStart,
    WorkflowStatus,
    DiagnosticsInvestigate,
    CommandInvoke,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectorResult {
    pub selector_id: String,
    pub status: SelectorStatus,
    #[serde(default)]
    pub action: Option<SelectorAction>,
    #[serde(default)]
    pub selected_workflow: Option<String>,
    #[serde(default)]
    pub diagnostics_scope: Option<Map<String, Value>>,
    #[serde(default)]
    pub function_id: Option<String>,
    #[serde(default)]
    pub function_args: Option<Map<String, Value>>,
    #[serde(default)]
    pub reason: Option<String>,
}

pub fn parse_and_validate_selector_result(
    raw_json: &str,
    request: &SelectorRequest,
) -> Result<SelectorResult, OrchestratorError> {
    let result: SelectorResult = serde_json::from_str(raw_json)
        .map_err(|e| OrchestratorError::SelectorJson(e.to_string()))?;

    if result.selector_id != request.selector_id {
        return Err(OrchestratorError::SelectorValidation(
            "selectorId mismatch".to_string(),
        ));
    }

    match result.status {
        SelectorStatus::Failed => Ok(result),
        SelectorStatus::Selected => {
            let action = result.action.ok_or_else(|| {
                OrchestratorError::SelectorValidation("selected result requires action".to_string())
            })?;
            match action {
                SelectorAction::WorkflowStart => {
                    let selected = result.selected_workflow.as_ref().ok_or_else(|| {
                        OrchestratorError::SelectorValidation(
                            "workflow_start requires selectedWorkflow".to_string(),
                        )
                    })?;
                    if !request.available_workflows.iter().any(|v| v == selected) {
                        return Err(OrchestratorError::SelectorValidation(format!(
                            "workflow `{selected}` is not in availableWorkflows"
                        )));
                    }
                }
                SelectorAction::WorkflowStatus => {}
                SelectorAction::DiagnosticsInvestigate => {
                    if result.diagnostics_scope.is_none() {
                        return Err(OrchestratorError::SelectorValidation(
                            "diagnostics_investigate requires diagnosticsScope object".to_string(),
                        ));
                    }
                }
                SelectorAction::CommandInvoke => {
                    let function_id = result.function_id.as_ref().ok_or_else(|| {
                        OrchestratorError::SelectorValidation(
                            "command_invoke requires functionId".to_string(),
                        )
                    })?;
                    if !request.available_functions.iter().any(|f| f == function_id) {
                        return Err(OrchestratorError::SelectorValidation(format!(
                            "function `{function_id}` is not in availableFunctions"
                        )));
                    }
                    if result.function_args.is_none() {
                        return Err(OrchestratorError::SelectorValidation(
                            "command_invoke requires functionArgs object".to_string(),
                        ));
                    }
                }
            }
            Ok(result)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionResolution {
    pub result: SelectorResult,
    pub retries_used: u32,
    pub fell_back_to_default_workflow: bool,
}

pub fn resolve_selector_with_retries<F>(
    orchestrator: &OrchestratorConfig,
    request: &SelectorRequest,
    mut next_attempt: F,
) -> SelectionResolution
where
    F: FnMut(u32) -> Option<String>,
{
    let mut attempt = 0_u32;
    while attempt < orchestrator.selection_max_retries {
        let raw = next_attempt(attempt);
        if let Some(raw) = raw {
            if let Ok(validated) = parse_and_validate_selector_result(&raw, request) {
                if validated.status == SelectorStatus::Selected {
                    return SelectionResolution {
                        result: validated,
                        retries_used: attempt,
                        fell_back_to_default_workflow: false,
                    };
                }
            }
        }
        attempt += 1;
    }

    SelectionResolution {
        result: SelectorResult {
            selector_id: request.selector_id.clone(),
            status: SelectorStatus::Selected,
            action: Some(SelectorAction::WorkflowStart),
            selected_workflow: Some(orchestrator.default_workflow.clone()),
            diagnostics_scope: None,
            function_id: None,
            function_args: None,
            reason: Some("fallback_to_default_workflow_after_retry_limit".to_string()),
        },
        retries_used: orchestrator.selection_max_retries,
        fell_back_to_default_workflow: true,
    }
}

pub struct SelectorArtifactStore {
    state_root: PathBuf,
}

impl SelectorArtifactStore {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
        }
    }

    pub fn persist_message_snapshot(
        &self,
        inbound: &IncomingMessage,
    ) -> Result<PathBuf, OrchestratorError> {
        let path = self
            .state_root
            .join("orchestrator/messages")
            .join(format!("{}.json", inbound.message_id));
        self.write_json(&path, inbound)
    }

    pub fn persist_selector_request(
        &self,
        request: &SelectorRequest,
    ) -> Result<PathBuf, OrchestratorError> {
        let path = self
            .state_root
            .join("orchestrator/select/incoming")
            .join(format!("{}.json", request.selector_id));
        self.write_json(&path, request)
    }

    pub fn move_request_to_processing(
        &self,
        selector_id: &str,
    ) -> Result<PathBuf, OrchestratorError> {
        let incoming = self
            .state_root
            .join("orchestrator/select/incoming")
            .join(format!("{selector_id}.json"));
        let processing = self
            .state_root
            .join("orchestrator/select/processing")
            .join(format!("{selector_id}.json"));
        fs::rename(&incoming, &processing).map_err(|e| io_error(&incoming, e))?;
        Ok(processing)
    }

    pub fn persist_selector_result(
        &self,
        result: &SelectorResult,
    ) -> Result<PathBuf, OrchestratorError> {
        let path = self
            .state_root
            .join("orchestrator/select/results")
            .join(format!("{}.json", result.selector_id));
        self.write_json(&path, result)
    }

    pub fn persist_selector_log(
        &self,
        selector_id: &str,
        content: &str,
    ) -> Result<PathBuf, OrchestratorError> {
        let path = self
            .state_root
            .join("orchestrator/select/logs")
            .join(format!("{selector_id}.log"));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        fs::write(&path, content).map_err(|e| io_error(&path, e))?;
        Ok(path)
    }

    fn write_json<T: Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<PathBuf, OrchestratorError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let body = serde_json::to_vec_pretty(value).map_err(|e| json_error(path, e))?;
        fs::write(path, body).map_err(|e| io_error(path, e))?;
        Ok(path.to_path_buf())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Queued,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Canceled,
}

impl RunState {
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (RunState::Queued, RunState::Running)
                | (RunState::Queued, RunState::Canceled)
                | (RunState::Running, RunState::Waiting)
                | (RunState::Running, RunState::Succeeded)
                | (RunState::Running, RunState::Failed)
                | (RunState::Running, RunState::Canceled)
                | (RunState::Waiting, RunState::Running)
                | (RunState::Waiting, RunState::Failed)
                | (RunState::Waiting, RunState::Canceled)
        )
    }
}

impl std::fmt::Display for RunState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunState::Queued => write!(f, "queued"),
            RunState::Running => write!(f, "running"),
            RunState::Waiting => write!(f, "waiting"),
            RunState::Succeeded => write!(f, "succeeded"),
            RunState::Failed => write!(f, "failed"),
            RunState::Canceled => write!(f, "canceled"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunRecord {
    pub run_id: String,
    pub workflow_id: String,
    pub state: RunState,
    #[serde(default)]
    pub current_step_id: Option<String>,
    #[serde(default)]
    pub current_attempt: Option<u32>,
    pub started_at: i64,
    pub updated_at: i64,
    pub total_iterations: u32,
    #[serde(default)]
    pub source_message_id: Option<String>,
    #[serde(default)]
    pub selector_id: Option<String>,
    #[serde(default)]
    pub selected_workflow: Option<String>,
    #[serde(default)]
    pub status_conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSnapshot {
    pub run_id: String,
    pub workflow_id: String,
    pub state: RunState,
    #[serde(default)]
    pub current_step_id: Option<String>,
    #[serde(default)]
    pub current_attempt: Option<u32>,
    pub started_at: i64,
    pub updated_at: i64,
    pub last_progress_at: i64,
    pub summary: String,
    pub pending_human_input: bool,
    pub next_expected_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepAttemptRecord {
    pub run_id: String,
    pub step_id: String,
    pub attempt: u32,
    pub started_at: i64,
    pub ended_at: i64,
    pub state: String,
    #[serde(default)]
    pub outputs: Map<String, Value>,
    #[serde(default)]
    pub next_step_id: Option<String>,
}

pub struct WorkflowRunStore {
    state_root: PathBuf,
}

impl WorkflowRunStore {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
        }
    }

    pub fn create_run(
        &self,
        run_id: impl Into<String>,
        workflow_id: impl Into<String>,
        now: i64,
    ) -> Result<WorkflowRunRecord, OrchestratorError> {
        let run = WorkflowRunRecord {
            run_id: run_id.into(),
            workflow_id: workflow_id.into(),
            state: RunState::Queued,
            current_step_id: None,
            current_attempt: None,
            started_at: now,
            updated_at: now,
            total_iterations: 0,
            source_message_id: None,
            selector_id: None,
            selected_workflow: None,
            status_conversation_id: None,
        };
        self.persist_run(&run)?;
        self.persist_progress(&ProgressSnapshot {
            run_id: run.run_id.clone(),
            workflow_id: run.workflow_id.clone(),
            state: run.state.clone(),
            current_step_id: None,
            current_attempt: None,
            started_at: run.started_at,
            updated_at: now,
            last_progress_at: now,
            summary: "queued".to_string(),
            pending_human_input: false,
            next_expected_action: "workflow start".to_string(),
        })?;
        Ok(run)
    }

    pub fn load_run(&self, run_id: &str) -> Result<WorkflowRunRecord, OrchestratorError> {
        let path = self.run_path(run_id);
        let raw = fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
        serde_json::from_str(&raw).map_err(|e| json_error(&path, e))
    }

    pub fn persist_run(&self, run: &WorkflowRunRecord) -> Result<(), OrchestratorError> {
        let path = self.run_path(&run.run_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let body = serde_json::to_vec_pretty(run).map_err(|e| json_error(&path, e))?;
        fs::write(&path, body).map_err(|e| io_error(&path, e))
    }

    pub fn transition_state(
        &self,
        run: &mut WorkflowRunRecord,
        next: RunState,
        now: i64,
        summary: impl Into<String>,
        pending_human_input: bool,
        next_expected_action: impl Into<String>,
    ) -> Result<(), OrchestratorError> {
        if !run.state.clone().can_transition_to(next.clone()) {
            return Err(OrchestratorError::InvalidRunTransition {
                from: run.state.clone(),
                to: next,
            });
        }
        run.state = next;
        run.updated_at = now;
        self.persist_run(run)?;
        self.persist_progress(&ProgressSnapshot {
            run_id: run.run_id.clone(),
            workflow_id: run.workflow_id.clone(),
            state: run.state.clone(),
            current_step_id: run.current_step_id.clone(),
            current_attempt: run.current_attempt,
            started_at: run.started_at,
            updated_at: now,
            last_progress_at: now,
            summary: summary.into(),
            pending_human_input,
            next_expected_action: next_expected_action.into(),
        })
    }

    pub fn persist_progress(&self, progress: &ProgressSnapshot) -> Result<(), OrchestratorError> {
        let path = self.progress_path(&progress.run_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let body = serde_json::to_vec_pretty(progress).map_err(|e| json_error(&path, e))?;
        fs::write(&path, body).map_err(|e| io_error(&path, e))
    }

    pub fn load_progress(&self, run_id: &str) -> Result<ProgressSnapshot, OrchestratorError> {
        let path = self.progress_path(run_id);
        let raw = fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
        serde_json::from_str(&raw).map_err(|e| json_error(&path, e))
    }

    pub fn persist_step_attempt(
        &self,
        attempt: &StepAttemptRecord,
    ) -> Result<PathBuf, OrchestratorError> {
        let path = self
            .run_dir(&attempt.run_id)
            .join("steps")
            .join(&attempt.step_id)
            .join(format!("attempt_{}.json", attempt.attempt));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let body = serde_json::to_vec_pretty(attempt).map_err(|e| json_error(&path, e))?;
        fs::write(&path, body).map_err(|e| io_error(&path, e))?;
        Ok(path)
    }

    fn run_dir(&self, run_id: &str) -> PathBuf {
        self.state_root.join("workflows/runs").join(run_id)
    }

    fn run_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("run.json")
    }

    fn progress_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("progress.json")
    }
}

pub fn parse_workflow_result_envelope(
    output: &str,
) -> Result<Map<String, Value>, OrchestratorError> {
    let open_tag = "[workflow_result]";
    let close_tag = "[/workflow_result]";
    let start = output.find(open_tag).ok_or_else(|| {
        OrchestratorError::WorkflowEnvelope("missing [workflow_result] tag".to_string())
    })?;
    let end = output.find(close_tag).ok_or_else(|| {
        OrchestratorError::WorkflowEnvelope("missing [/workflow_result] tag".to_string())
    })?;
    if end <= start {
        return Err(OrchestratorError::WorkflowEnvelope(
            "invalid workflow_result tag ordering".to_string(),
        ));
    }
    let json_str = output[start + open_tag.len()..end].trim();
    let value: Value = serde_json::from_str(json_str)
        .map_err(|e| OrchestratorError::WorkflowEnvelope(format!("invalid json: {e}")))?;
    let obj = value.as_object().ok_or_else(|| {
        OrchestratorError::WorkflowEnvelope(
            "workflow_result payload must be a JSON object".to_string(),
        )
    })?;
    Ok(obj.clone())
}

pub fn parse_review_decision(outputs: &Map<String, Value>) -> Result<bool, OrchestratorError> {
    let decision = outputs
        .get("decision")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    match decision.to_ascii_lowercase().as_str() {
        "approve" => Ok(true),
        "reject" => Ok(false),
        other => Err(OrchestratorError::InvalidReviewDecision(other.to_string())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEvaluation {
    pub outputs: Map<String, Value>,
    pub next_step_id: Option<String>,
}

pub fn evaluate_step_result(
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
    raw_output: &str,
) -> Result<StepEvaluation, OrchestratorError> {
    let parsed = parse_workflow_result_envelope(raw_output)?;
    if step.step_type == "agent_review" {
        let approve = parse_review_decision(&parsed)?;
        let next = if approve {
            step.on_approve.clone()
        } else {
            step.on_reject.clone()
        };
        return Ok(StepEvaluation {
            outputs: parsed,
            next_step_id: next,
        });
    }

    let next = step
        .next
        .clone()
        .or_else(|| next_step_in_workflow(workflow, &step.id));

    Ok(StepEvaluation {
        outputs: parsed,
        next_step_id: next,
    })
}

fn next_step_in_workflow(workflow: &WorkflowConfig, step_id: &str) -> Option<String> {
    workflow
        .steps
        .iter()
        .position(|s| s.id == step_id)
        .and_then(|idx| workflow.steps.get(idx + 1))
        .map(|s| s.id.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionSafetyLimits {
    pub max_total_iterations: u32,
    pub run_timeout_seconds: u64,
    pub step_timeout_seconds: u64,
    pub max_retries: u32,
}

impl Default for ExecutionSafetyLimits {
    fn default() -> Self {
        Self {
            max_total_iterations: 12,
            run_timeout_seconds: 3600,
            step_timeout_seconds: 900,
            max_retries: 2,
        }
    }
}

pub fn enforce_execution_safety(
    run: &WorkflowRunRecord,
    limits: ExecutionSafetyLimits,
    now: i64,
    current_step_started_at: i64,
    current_attempt: u32,
) -> Result<(), OrchestratorError> {
    if run.total_iterations >= limits.max_total_iterations {
        return Err(OrchestratorError::MaxIterationsExceeded {
            max_total_iterations: limits.max_total_iterations,
        });
    }

    if now.saturating_sub(run.started_at) > limits.run_timeout_seconds as i64 {
        return Err(OrchestratorError::RunTimeout {
            run_timeout_seconds: limits.run_timeout_seconds,
        });
    }

    if now.saturating_sub(current_step_started_at) > limits.step_timeout_seconds as i64 {
        return Err(OrchestratorError::StepTimeout {
            step_timeout_seconds: limits.step_timeout_seconds,
        });
    }

    if current_attempt > limits.max_retries + 1 {
        return Err(OrchestratorError::SelectorValidation(format!(
            "retry limit exceeded for step attempt {current_attempt} (max retries {})",
            limits.max_retries
        )));
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCall {
    pub function_id: String,
    pub args: Map<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct FunctionRegistry {
    allowed: BTreeSet<String>,
}

impl FunctionRegistry {
    pub fn new<I>(function_ids: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        Self {
            allowed: function_ids.into_iter().collect(),
        }
    }

    pub fn contains(&self, function_id: &str) -> bool {
        self.allowed.contains(function_id)
    }

    pub fn invoke(&self, call: &FunctionCall) -> Result<Value, OrchestratorError> {
        if !self.contains(&call.function_id) {
            return Err(OrchestratorError::UnknownFunction {
                function_id: call.function_id.clone(),
            });
        }
        Ok(Value::Object(Map::from_iter([
            (
                "functionId".to_string(),
                Value::String(call.function_id.clone()),
            ),
            ("functionArgs".to_string(), Value::Object(call.args.clone())),
        ])))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResolutionInput {
    pub explicit_run_id: Option<String>,
    pub inbound_workflow_run_id: Option<String>,
    pub channel_profile_id: Option<String>,
    pub conversation_id: Option<String>,
}

pub fn resolve_status_run_id(
    input: &StatusResolutionInput,
    active_conversation_runs: &BTreeMap<(String, String), String>,
) -> Option<String> {
    if let Some(explicit) = input
        .explicit_run_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return Some(explicit.clone());
    }

    if let Some(inbound) = input
        .inbound_workflow_run_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return Some(inbound.clone());
    }

    let key = (
        input.channel_profile_id.as_ref()?.to_string(),
        input.conversation_id.as_ref()?.to_string(),
    );
    active_conversation_runs.get(&key).cloned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutedSelectorAction {
    WorkflowStart { workflow_id: String },
    WorkflowStatus { run_id: Option<String> },
    CommandInvoke { result: Value },
}

pub fn route_selector_action(
    request: &SelectorRequest,
    result: &SelectorResult,
    status_input: &StatusResolutionInput,
    active_conversation_runs: &BTreeMap<(String, String), String>,
    functions: &FunctionRegistry,
) -> Result<RoutedSelectorAction, OrchestratorError> {
    let validated = parse_and_validate_selector_result(
        &serde_json::to_string(result)
            .map_err(|e| OrchestratorError::SelectorJson(e.to_string()))?,
        request,
    )?;

    let action = validated
        .action
        .ok_or_else(|| OrchestratorError::SelectorValidation("missing action".to_string()))?;
    match action {
        SelectorAction::WorkflowStart => Ok(RoutedSelectorAction::WorkflowStart {
            workflow_id: validated.selected_workflow.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow_start requires selectedWorkflow".to_string(),
                )
            })?,
        }),
        SelectorAction::WorkflowStatus => Ok(RoutedSelectorAction::WorkflowStatus {
            run_id: resolve_status_run_id(status_input, active_conversation_runs),
        }),
        SelectorAction::CommandInvoke => {
            let function_id = validated.function_id.ok_or_else(|| {
                OrchestratorError::SelectorValidation("missing functionId".to_string())
            })?;
            let function_args = validated.function_args.unwrap_or_default();
            let call = FunctionCall {
                function_id,
                args: function_args,
            };
            let invoke_result = functions.invoke(&call)?;
            Ok(RoutedSelectorAction::CommandInvoke {
                result: invoke_result,
            })
        }
        SelectorAction::DiagnosticsInvestigate => Err(OrchestratorError::SelectorValidation(
            "diagnostics_investigate is not part of this phase action router".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    #[test]
    fn resolve_orchestrator_id_from_channel_profile() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspace_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  eng:
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: eng
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {}
channels: {}
"#,
        )
        .expect("settings");

        let inbound = IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("engineering".to_string()),
            sender: "dana".to_string(),
            sender_id: "U42".to_string(),
            message: "status?".to_string(),
            timestamp: 1,
            message_id: "m1".to_string(),
            conversation_id: Some("c1".to_string()),
            files: vec![],
            workflow_run_id: None,
            workflow_step_id: None,
        };

        let resolved = resolve_orchestrator_id(&settings, &inbound).expect("resolved");
        assert_eq!(resolved, "eng");
    }

    #[test]
    fn selector_validation_rejects_unknown_function() {
        let request = SelectorRequest {
            selector_id: "sel-1".to_string(),
            channel_profile_id: "engineering".to_string(),
            message_id: "m1".to_string(),
            conversation_id: Some("c1".to_string()),
            user_message: "run command".to_string(),
            available_workflows: vec!["wf".to_string()],
            default_workflow: "wf".to_string(),
            available_functions: vec!["workflow.status".to_string()],
        };
        let raw = r#"{
          "selectorId":"sel-1",
          "status":"selected",
          "action":"command_invoke",
          "functionId":"workflow.cancel",
          "functionArgs":{}
        }"#;
        let err = parse_and_validate_selector_result(raw, &request).expect_err("must fail");
        assert!(err.to_string().contains("availableFunctions"));
    }

    #[test]
    fn workflow_result_envelope_parse_and_review_decision() {
        let raw = r#"
ignored
[workflow_result]
{"decision":"approve","feedback":"ok"}
[/workflow_result]
"#;
        let parsed = parse_workflow_result_envelope(raw).expect("parsed");
        let decision = parse_review_decision(&parsed).expect("decision");
        assert!(decision);
    }

    #[test]
    fn run_state_transition_guards_work() {
        assert!(RunState::Queued.can_transition_to(RunState::Running));
        assert!(!RunState::Succeeded.can_transition_to(RunState::Running));
        assert!(!RunState::Failed.can_transition_to(RunState::Running));
    }
}
