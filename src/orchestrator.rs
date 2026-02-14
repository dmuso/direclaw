use crate::config::{
    load_orchestrator_config, AgentConfig, ConfigError, OrchestratorConfig, Settings,
    WorkflowConfig, WorkflowStepConfig,
};
use crate::queue::IncomingMessage;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

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
    #[error("missing required function argument `{arg}`")]
    MissingFunctionArg { arg: String },
    #[error("workflow run `{run_id}` not found")]
    UnknownRunId { run_id: String },
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
    #[error("workspace access denied for orchestrator `{orchestrator_id}` at path `{path}`")]
    WorkspaceAccessDenied {
        orchestrator_id: String,
        path: String,
    },
    #[error("workspace path validation failed for `{path}`: {reason}")]
    WorkspacePathValidation { path: String, reason: String },
    #[error("output path validation failed for step `{step_id}` template `{template}`: {reason}")]
    OutputPathValidation {
        step_id: String,
        template: String,
        reason: String,
    },
    #[error("config error: {0}")]
    Config(String),
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

impl From<ConfigError> for OrchestratorError {
    fn from(value: ConfigError) -> Self {
        Self::Config(value.to_string())
    }
}

fn io_error(path: &Path, source: std::io::Error) -> OrchestratorError {
    OrchestratorError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn append_security_log(state_root: &Path, line: &str) {
    let path = state_root.join("logs/security.log");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = file.write_all(format!("{line}\n").as_bytes());
}

fn json_error(path: &Path, source: serde_json::Error) -> OrchestratorError {
    OrchestratorError::Json {
        path: path.display().to_string(),
        source,
    }
}

fn missing_run_for_io(run_id: &str, err: &OrchestratorError) -> Option<OrchestratorError> {
    match err {
        OrchestratorError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            Some(OrchestratorError::UnknownRunId {
                run_id: run_id.to_string(),
            })
        }
        _ => None,
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
    let max_attempts = orchestrator.selection_max_retries.saturating_add(1);
    let mut attempt = 0_u32;
    while attempt < max_attempts {
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
        if let Some(parent) = processing.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
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

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Succeeded | RunState::Failed | RunState::Canceled
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

#[derive(Debug, Clone)]
pub struct WorkflowRunStore {
    state_root: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectorStartedRunMetadata {
    pub source_message_id: Option<String>,
    pub selector_id: Option<String>,
    pub selected_workflow: Option<String>,
    pub status_conversation_id: Option<String>,
}

impl WorkflowRunStore {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
        }
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub fn create_run(
        &self,
        run_id: impl Into<String>,
        workflow_id: impl Into<String>,
        now: i64,
    ) -> Result<WorkflowRunRecord, OrchestratorError> {
        self.create_run_with_metadata(
            run_id,
            workflow_id,
            SelectorStartedRunMetadata::default(),
            now,
        )
    }

    pub fn create_run_with_metadata(
        &self,
        run_id: impl Into<String>,
        workflow_id: impl Into<String>,
        metadata: SelectorStartedRunMetadata,
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
            source_message_id: metadata.source_message_id,
            selector_id: metadata.selector_id,
            selected_workflow: metadata.selected_workflow,
            status_conversation_id: metadata.status_conversation_id,
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
        let path = self.run_metadata_path(run_id);
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let legacy = self.run_dir(run_id).join("run.json");
                fs::read_to_string(&legacy).map_err(|e| io_error(&legacy, e))?
            }
            Err(err) => return Err(io_error(&path, err)),
        };
        serde_json::from_str(&raw).map_err(|e| json_error(&path, e))
    }

    pub fn persist_run(&self, run: &WorkflowRunRecord) -> Result<(), OrchestratorError> {
        let path = self.run_metadata_path(&run.run_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let body = serde_json::to_vec_pretty(run).map_err(|e| json_error(&path, e))?;
        fs::write(&path, &body).map_err(|e| io_error(&path, e))?;

        // Keep a mirrored run record in run directory for compatibility.
        let legacy_path = self.run_dir(&run.run_id).join("run.json");
        if let Some(parent) = legacy_path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        fs::write(&legacy_path, body).map_err(|e| io_error(&legacy_path, e))
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

    pub fn heartbeat_tick(
        &self,
        run: &WorkflowRunRecord,
        now: i64,
        summary: impl Into<String>,
    ) -> Result<(), OrchestratorError> {
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
            pending_human_input: run.state == RunState::Waiting,
            next_expected_action: if run.state == RunState::Waiting {
                "await human response".to_string()
            } else {
                "continue workflow".to_string()
            },
        })
    }

    pub fn mark_step_attempt_started(
        &self,
        run: &mut WorkflowRunRecord,
        step_id: &str,
        attempt: u32,
        now: i64,
    ) -> Result<(), OrchestratorError> {
        run.current_step_id = Some(step_id.to_string());
        run.current_attempt = Some(attempt);
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
            summary: format!("step {step_id} attempt {attempt} running"),
            pending_human_input: false,
            next_expected_action: "await step output".to_string(),
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
            .join("attempts")
            .join(attempt.attempt.to_string())
            .join("result.json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let body = serde_json::to_vec_pretty(attempt).map_err(|e| json_error(&path, e))?;
        fs::write(&path, body).map_err(|e| io_error(&path, e))?;

        let mut run = self.load_run(&attempt.run_id)?;
        run.total_iterations = run.total_iterations.saturating_add(1);
        run.updated_at = attempt.ended_at;
        self.persist_run(&run)?;
        self.persist_progress(&ProgressSnapshot {
            run_id: run.run_id.clone(),
            workflow_id: run.workflow_id.clone(),
            state: run.state.clone(),
            current_step_id: Some(attempt.step_id.clone()),
            current_attempt: Some(attempt.attempt),
            started_at: run.started_at,
            updated_at: attempt.ended_at,
            last_progress_at: attempt.ended_at,
            summary: format!(
                "step {} attempt {} {}",
                attempt.step_id, attempt.attempt, attempt.state
            ),
            pending_human_input: run.state == RunState::Waiting,
            next_expected_action: attempt
                .next_step_id
                .clone()
                .unwrap_or_else(|| "workflow terminal transition".to_string()),
        })?;

        Ok(path)
    }

    fn run_dir(&self, run_id: &str) -> PathBuf {
        self.state_root.join("workflows/runs").join(run_id)
    }

    fn run_metadata_path(&self, run_id: &str) -> PathBuf {
        self.state_root
            .join("workflows/runs")
            .join(format!("{run_id}.json"))
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

pub fn resolve_execution_safety_limits(
    orchestrator: &OrchestratorConfig,
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
) -> ExecutionSafetyLimits {
    let defaults = ExecutionSafetyLimits::default();
    let orchestration = orchestrator.workflow_orchestration.as_ref();

    let mut step_timeout_seconds = orchestration
        .and_then(|v| v.default_step_timeout_seconds)
        .unwrap_or(defaults.step_timeout_seconds);
    if let Some(max_step) = orchestration.and_then(|v| v.max_step_timeout_seconds) {
        step_timeout_seconds = step_timeout_seconds.min(max_step);
    }

    ExecutionSafetyLimits {
        max_total_iterations: workflow
            .limits
            .as_ref()
            .and_then(|v| v.max_total_iterations)
            .or_else(|| orchestration.and_then(|v| v.max_total_iterations))
            .unwrap_or(defaults.max_total_iterations),
        run_timeout_seconds: workflow
            .limits
            .as_ref()
            .and_then(|v| v.run_timeout_seconds)
            .or_else(|| orchestration.and_then(|v| v.default_run_timeout_seconds))
            .unwrap_or(defaults.run_timeout_seconds),
        step_timeout_seconds,
        max_retries: step
            .limits
            .as_ref()
            .and_then(|v| v.max_retries)
            .unwrap_or(defaults.max_retries),
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
pub struct WorkspaceAccessContext {
    pub orchestrator_id: String,
    pub private_workspace_root: PathBuf,
    pub shared_workspace_roots: BTreeMap<String, PathBuf>,
}

pub fn resolve_workspace_access_context(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<WorkspaceAccessContext, OrchestratorError> {
    let private_workspace = canonicalize_absolute_path_if_exists(
        &settings.resolve_private_workspace(orchestrator_id)?,
    )?;
    let orchestrator = settings.orchestrators.get(orchestrator_id).ok_or_else(|| {
        OrchestratorError::Config(format!(
            "missing orchestrator `{orchestrator_id}` in settings"
        ))
    })?;

    let mut shared_workspace_roots = BTreeMap::new();
    for grant in &orchestrator.shared_access {
        let shared = settings.shared_workspaces.get(grant).ok_or_else(|| {
            OrchestratorError::Config(format!(
                "orchestrator `{orchestrator_id}` references unknown shared workspace `{grant}`"
            ))
        })?;
        let normalized = canonicalize_absolute_path_if_exists(shared)?;
        shared_workspace_roots.insert(grant.clone(), normalized);
    }

    Ok(WorkspaceAccessContext {
        orchestrator_id: orchestrator_id.to_string(),
        private_workspace_root: private_workspace,
        shared_workspace_roots,
    })
}

pub fn enforce_workspace_access(
    context: &WorkspaceAccessContext,
    requested_paths: &[PathBuf],
) -> Result<(), OrchestratorError> {
    for requested in requested_paths {
        let normalized = canonicalize_absolute_path_if_exists(requested)?;
        if normalized.starts_with(&context.private_workspace_root) {
            continue;
        }
        if context
            .shared_workspace_roots
            .values()
            .any(|root| normalized.starts_with(root))
        {
            continue;
        }
        return Err(OrchestratorError::WorkspaceAccessDenied {
            orchestrator_id: context.orchestrator_id.clone(),
            path: normalized.display().to_string(),
        });
    }
    Ok(())
}

fn canonicalize_absolute_path_if_exists(path: &Path) -> Result<PathBuf, OrchestratorError> {
    match fs::canonicalize(path) {
        Ok(canonical) => normalize_absolute_path(&canonical),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => normalize_absolute_path(path),
        Err(err) => Err(io_error(path, err)),
    }
}

fn resolve_agent_workspace_root(
    private_workspace_root: &Path,
    agent_id: &str,
    agent: &AgentConfig,
) -> PathBuf {
    match &agent.private_workspace {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => private_workspace_root.join(path),
        None => private_workspace_root.join("agents").join(agent_id),
    }
}

fn collect_orchestrator_requested_paths(
    context: &WorkspaceAccessContext,
    settings: &Settings,
    orchestrator: &OrchestratorConfig,
) -> Result<Vec<PathBuf>, OrchestratorError> {
    let mut requested = vec![context.private_workspace_root.clone()];
    for (agent_id, agent) in &orchestrator.agents {
        requested.push(resolve_agent_workspace_root(
            &context.private_workspace_root,
            agent_id,
            agent,
        ));
        for shared in &agent.shared_access {
            let path = settings.shared_workspaces.get(shared).ok_or_else(|| {
                OrchestratorError::Config(format!(
                    "agent `{agent_id}` references unknown shared workspace `{shared}`"
                ))
            })?;
            requested.push(path.clone());
        }
    }
    Ok(requested)
}

fn validate_selected_workflow_output_paths(
    state_root: &Path,
    run_id: &str,
    orchestrator: &OrchestratorConfig,
    workflow_id: &str,
) -> Result<(), OrchestratorError> {
    let workflow = orchestrator
        .workflows
        .iter()
        .find(|w| w.id == workflow_id)
        .ok_or_else(|| {
            OrchestratorError::SelectorValidation(format!(
                "workflow `{workflow_id}` is not declared in orchestrator"
            ))
        })?;
    for step in &workflow.steps {
        let _ = resolve_step_output_paths(state_root, run_id, step, 1)?;
    }
    Ok(())
}

pub fn interpolate_output_template(
    template: &str,
    run_id: &str,
    step_id: &str,
    attempt: u32,
) -> String {
    template
        .replace("{{workflow.run_id}}", run_id)
        .replace("{{workflow.step_id}}", step_id)
        .replace("{{workflow.attempt}}", &attempt.to_string())
}

pub fn resolve_step_output_paths(
    state_root: &Path,
    run_id: &str,
    step: &WorkflowStepConfig,
    attempt: u32,
) -> Result<BTreeMap<String, PathBuf>, OrchestratorError> {
    let output_root = normalize_absolute_path(
        &state_root
            .join("workflows/runs")
            .join(run_id)
            .join("steps")
            .join(&step.id)
            .join("attempts")
            .join(attempt.to_string())
            .join("outputs"),
    )?;

    let mut output_paths = BTreeMap::new();
    let Some(templates) = step.output_files.as_ref() else {
        return Ok(output_paths);
    };

    for (key, template) in templates {
        let interpolated = interpolate_output_template(template, run_id, &step.id, attempt);
        let relative = validate_relative_output_template(&interpolated, &step.id, template)?;
        let resolved = normalize_absolute_path(&output_root.join(relative))?;
        if !resolved.starts_with(&output_root) {
            return Err(OrchestratorError::OutputPathValidation {
                step_id: step.id.clone(),
                template: template.clone(),
                reason: format!(
                    "resolved path `{}` escapes output root `{}`",
                    resolved.display(),
                    output_root.display()
                ),
            });
        }
        output_paths.insert(key.clone(), resolved);
    }
    Ok(output_paths)
}

fn validate_relative_output_template<'a>(
    interpolated: &'a str,
    step_id: &str,
    template: &str,
) -> Result<&'a Path, OrchestratorError> {
    let relative = Path::new(interpolated);
    if relative.is_absolute() {
        return Err(OrchestratorError::OutputPathValidation {
            step_id: step_id.to_string(),
            template: template.to_string(),
            reason: "output path template must be relative".to_string(),
        });
    }

    let mut has_normal = false;
    for component in relative.components() {
        match component {
            Component::Normal(_) => has_normal = true,
            Component::CurDir | Component::ParentDir => {
                return Err(OrchestratorError::OutputPathValidation {
                    step_id: step_id.to_string(),
                    template: template.to_string(),
                    reason: "non-canonical relative segments (`.` or `..`) are not allowed"
                        .to_string(),
                })
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(OrchestratorError::OutputPathValidation {
                    step_id: step_id.to_string(),
                    template: template.to_string(),
                    reason: "absolute-style segments are not allowed".to_string(),
                })
            }
        }
    }

    if !has_normal {
        return Err(OrchestratorError::OutputPathValidation {
            step_id: step_id.to_string(),
            template: template.to_string(),
            reason: "output template must resolve to a file path".to_string(),
        });
    }

    Ok(relative)
}

fn normalize_absolute_path(path: &Path) -> Result<PathBuf, OrchestratorError> {
    if !path.is_absolute() {
        return Err(OrchestratorError::WorkspacePathValidation {
            path: path.display().to_string(),
            reason: "path must be absolute".to_string(),
        });
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(v) => normalized.push(v),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(OrchestratorError::WorkspacePathValidation {
                        path: path.display().to_string(),
                        reason: "path escapes filesystem root".to_string(),
                    });
                }
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }
    Ok(normalized)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCall {
    pub function_id: String,
    pub args: Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct FunctionRegistry {
    allowed: BTreeSet<String>,
    run_store: Option<WorkflowRunStore>,
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self::new(Vec::<String>::new())
    }
}

impl FunctionRegistry {
    pub fn new<I>(function_ids: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        Self {
            allowed: function_ids.into_iter().collect(),
            run_store: None,
        }
    }

    pub fn with_run_store<I>(function_ids: I, run_store: WorkflowRunStore) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        Self {
            allowed: function_ids.into_iter().collect(),
            run_store: Some(run_store),
        }
    }

    pub fn contains(&self, function_id: &str) -> bool {
        self.allowed.contains(function_id)
    }

    pub fn available_function_ids(&self) -> Vec<String> {
        self.allowed.iter().cloned().collect()
    }

    pub fn invoke(&self, call: &FunctionCall) -> Result<Value, OrchestratorError> {
        if !self.contains(&call.function_id) {
            return Err(OrchestratorError::UnknownFunction {
                function_id: call.function_id.clone(),
            });
        }

        match call.function_id.as_str() {
            "workflow.status" => {
                let run_id = parse_run_id_arg(&call.args)?;
                let run_store = self.run_store.as_ref().ok_or_else(|| {
                    OrchestratorError::SelectorValidation(
                        "workflow.status requires workflow run store".to_string(),
                    )
                })?;
                let progress = run_store
                    .load_progress(&run_id)
                    .map_err(|e| missing_run_for_io(&run_id, &e).unwrap_or(e))?;
                Ok(Value::Object(Map::from_iter([
                    ("runId".to_string(), Value::String(run_id)),
                    (
                        "progress".to_string(),
                        serde_json::to_value(progress)
                            .map_err(|e| OrchestratorError::SelectorJson(e.to_string()))?,
                    ),
                ])))
            }
            "workflow.cancel" => {
                let run_id = parse_run_id_arg(&call.args)?;
                let run_store = self.run_store.as_ref().ok_or_else(|| {
                    OrchestratorError::SelectorValidation(
                        "workflow.cancel requires workflow run store".to_string(),
                    )
                })?;
                let mut run = run_store
                    .load_run(&run_id)
                    .map_err(|e| missing_run_for_io(&run_id, &e).unwrap_or(e))?;
                if !run.state.clone().is_terminal() {
                    let now = run.updated_at.saturating_add(1);
                    run_store.transition_state(
                        &mut run,
                        RunState::Canceled,
                        now,
                        "canceled by command",
                        false,
                        "none",
                    )?;
                }
                Ok(Value::Object(Map::from_iter([
                    ("runId".to_string(), Value::String(run_id)),
                    ("state".to_string(), Value::String(run.state.to_string())),
                ])))
            }
            "orchestrator.list" => Ok(Value::Object(Map::from_iter([(
                "availableFunctions".to_string(),
                Value::Array(
                    self.allowed
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect::<Vec<_>>(),
                ),
            )]))),
            _ => Err(OrchestratorError::SelectorValidation(format!(
                "function `{}` is allowed but has no implementation",
                call.function_id
            ))),
        }
    }
}

fn parse_run_id_arg(args: &Map<String, Value>) -> Result<String, OrchestratorError> {
    args.get("runId")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| OrchestratorError::MissingFunctionArg {
            arg: "runId".to_string(),
        })
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
    WorkflowStart {
        run_id: String,
        workflow_id: String,
    },
    WorkflowStatus {
        run_id: Option<String>,
        progress: Option<ProgressSnapshot>,
        message: String,
    },
    DiagnosticsInvestigate {
        run_id: Option<String>,
        findings: String,
    },
    CommandInvoke {
        result: Value,
    },
}

pub struct RouteContext<'a> {
    pub status_input: &'a StatusResolutionInput,
    pub active_conversation_runs: &'a BTreeMap<(String, String), String>,
    pub functions: &'a FunctionRegistry,
    pub run_store: &'a WorkflowRunStore,
    pub orchestrator: &'a OrchestratorConfig,
    pub source_message_id: Option<&'a str>,
    pub now: i64,
}

pub fn route_selector_action(
    request: &SelectorRequest,
    result: &SelectorResult,
    ctx: RouteContext<'_>,
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
        SelectorAction::WorkflowStart => {
            let workflow_id = validated.selected_workflow.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow_start requires selectedWorkflow".to_string(),
                )
            })?;
            if !ctx
                .orchestrator
                .workflows
                .iter()
                .any(|w| w.id == workflow_id)
            {
                let err = OrchestratorError::SelectorValidation(format!(
                    "workflow `{workflow_id}` is not declared in orchestrator"
                ));
                append_security_log(
                    ctx.run_store.state_root(),
                    &format!("workflow_start denied: {err}"),
                );
                return Err(err);
            }

            let run_id = format!("run-{}-{}", request.selector_id, ctx.now);
            if let Err(err) = validate_selected_workflow_output_paths(
                ctx.run_store.state_root(),
                &run_id,
                ctx.orchestrator,
                &workflow_id,
            ) {
                append_security_log(
                    ctx.run_store.state_root(),
                    &format!(
                        "output path validation denied for workflow `{workflow_id}` run `{run_id}`: {err}"
                    ),
                );
                return Err(err);
            }
            let mut run = ctx.run_store.create_run_with_metadata(
                run_id.clone(),
                workflow_id.clone(),
                SelectorStartedRunMetadata {
                    source_message_id: ctx.source_message_id.map(|v| v.to_string()),
                    selector_id: Some(request.selector_id.clone()),
                    selected_workflow: Some(workflow_id.clone()),
                    status_conversation_id: request.conversation_id.clone(),
                },
                ctx.now,
            )?;
            ctx.run_store.transition_state(
                &mut run,
                RunState::Running,
                ctx.now,
                format!("workflow {workflow_id} started"),
                false,
                "execute first step",
            )?;

            Ok(RoutedSelectorAction::WorkflowStart {
                run_id,
                workflow_id,
            })
        }
        SelectorAction::WorkflowStatus => {
            let run_id = resolve_status_run_id(ctx.status_input, ctx.active_conversation_runs);
            let Some(run_id_value) = run_id.clone() else {
                return Ok(RoutedSelectorAction::WorkflowStatus {
                    run_id: None,
                    progress: None,
                    message: "no active workflow run found for this conversation".to_string(),
                });
            };

            match ctx.run_store.load_progress(&run_id_value) {
                Ok(progress) => Ok(RoutedSelectorAction::WorkflowStatus {
                    run_id,
                    progress: Some(progress),
                    message: "workflow progress loaded".to_string(),
                }),
                Err(err) => {
                    let err = missing_run_for_io(&run_id_value, &err).unwrap_or(err);
                    match err {
                        OrchestratorError::UnknownRunId { .. } => {
                            Ok(RoutedSelectorAction::WorkflowStatus {
                                run_id,
                                progress: None,
                                message: format!("workflow run `{run_id_value}` was not found"),
                            })
                        }
                        other => Err(other),
                    }
                }
            }
        }
        SelectorAction::DiagnosticsInvestigate => {
            let explicit_run = validated
                .diagnostics_scope
                .as_ref()
                .and_then(|m| m.get("runId"))
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string());
            let run_id = explicit_run
                .or_else(|| ctx.status_input.inbound_workflow_run_id.clone())
                .or_else(|| {
                    if let (Some(profile), Some(conv)) = (
                        ctx.status_input.channel_profile_id.as_ref(),
                        ctx.status_input.conversation_id.as_ref(),
                    ) {
                        ctx.active_conversation_runs
                            .get(&(profile.clone(), conv.clone()))
                            .cloned()
                    } else {
                        None
                    }
                });

            let diagnostics_id = format!("diag-{}-{}", request.selector_id, ctx.now);
            let diagnostics_root = ctx.run_store.state_root().join("orchestrator/diagnostics");
            fs::create_dir_all(diagnostics_root.join("context"))
                .map_err(|e| io_error(&diagnostics_root, e))?;
            fs::create_dir_all(diagnostics_root.join("results"))
                .map_err(|e| io_error(&diagnostics_root, e))?;
            fs::create_dir_all(diagnostics_root.join("logs"))
                .map_err(|e| io_error(&diagnostics_root, e))?;

            let (findings, context_bundle) = if let Some(run_id_value) = run_id.clone() {
                match ctx.run_store.load_progress(&run_id_value) {
                    Ok(progress) => (
                        format!(
                            "Diagnostics summary for run {}: state={}, summary={}.",
                            run_id_value, progress.state, progress.summary
                        ),
                        Value::Object(Map::from_iter([
                            ("diagnosticsId".to_string(), Value::String(diagnostics_id.clone())),
                            ("runId".to_string(), Value::String(run_id_value.clone())),
                            (
                                "progress".to_string(),
                                serde_json::to_value(progress)
                                    .map_err(|e| OrchestratorError::SelectorJson(e.to_string()))?,
                            ),
                        ])),
                    ),
                    Err(_) => (
                        format!(
                            "Requested diagnostics for run `{run_id_value}`, but no persisted progress was found."
                        ),
                        Value::Object(Map::from_iter([
                            ("diagnosticsId".to_string(), Value::String(diagnostics_id.clone())),
                            ("runId".to_string(), Value::String(run_id_value)),
                            (
                                "note".to_string(),
                                Value::String("run artifacts not found".to_string()),
                            ),
                        ])),
                    ),
                }
            } else {
                (
                    "Diagnostics scope is ambiguous. Which workflow run should I investigate?"
                        .to_string(),
                    Value::Object(Map::from_iter([
                        (
                            "diagnosticsId".to_string(),
                            Value::String(diagnostics_id.clone()),
                        ),
                        (
                            "note".to_string(),
                            Value::String("scope unresolved".to_string()),
                        ),
                    ])),
                )
            };

            let context_path = diagnostics_root
                .join("context")
                .join(format!("{diagnostics_id}.json"));
            let result_path = diagnostics_root
                .join("results")
                .join(format!("{diagnostics_id}.json"));
            let log_path = diagnostics_root
                .join("logs")
                .join(format!("{diagnostics_id}.log"));

            fs::write(
                &context_path,
                serde_json::to_vec_pretty(&context_bundle)
                    .map_err(|e| json_error(&context_path, e))?,
            )
            .map_err(|e| io_error(&context_path, e))?;

            fs::write(
                &result_path,
                serde_json::to_vec_pretty(&Value::Object(Map::from_iter([
                    (
                        "diagnosticsId".to_string(),
                        Value::String(diagnostics_id.clone()),
                    ),
                    ("findings".to_string(), Value::String(findings.clone())),
                ])))
                .map_err(|e| json_error(&result_path, e))?,
            )
            .map_err(|e| io_error(&result_path, e))?;

            fs::write(&log_path, findings.as_bytes()).map_err(|e| io_error(&log_path, e))?;

            Ok(RoutedSelectorAction::DiagnosticsInvestigate { run_id, findings })
        }
        SelectorAction::CommandInvoke => {
            let function_id = validated.function_id.ok_or_else(|| {
                OrchestratorError::SelectorValidation("missing functionId".to_string())
            })?;
            let function_args = validated.function_args.unwrap_or_default();
            let call = FunctionCall {
                function_id,
                args: function_args,
            };
            let invoke_result = ctx.functions.invoke(&call)?;
            Ok(RoutedSelectorAction::CommandInvoke {
                result: invoke_result,
            })
        }
    }
}

pub fn process_queued_message<F>(
    state_root: &Path,
    settings: &Settings,
    inbound: &IncomingMessage,
    now: i64,
    active_conversation_runs: &BTreeMap<(String, String), String>,
    functions: &FunctionRegistry,
    mut next_selector_attempt: F,
) -> Result<RoutedSelectorAction, OrchestratorError>
where
    F: FnMut(u32, &SelectorRequest, &OrchestratorConfig) -> Option<String>,
{
    let run_store = WorkflowRunStore::new(state_root);
    if let Some(run_id) = inbound
        .workflow_run_id
        .as_ref()
        .filter(|v| !v.trim().is_empty())
    {
        let status_input = StatusResolutionInput {
            explicit_run_id: Some(run_id.clone()),
            inbound_workflow_run_id: Some(run_id.clone()),
            channel_profile_id: inbound.channel_profile_id.clone(),
            conversation_id: inbound.conversation_id.clone(),
        };

        let pseudo_request = SelectorRequest {
            selector_id: format!("status-{}", inbound.message_id),
            channel_profile_id: inbound.channel_profile_id.clone().unwrap_or_default(),
            message_id: inbound.message_id.clone(),
            conversation_id: inbound.conversation_id.clone(),
            user_message: inbound.message.clone(),
            available_workflows: Vec::new(),
            default_workflow: String::new(),
            available_functions: functions.available_function_ids(),
        };
        let status_result = SelectorResult {
            selector_id: pseudo_request.selector_id.clone(),
            status: SelectorStatus::Selected,
            action: Some(SelectorAction::WorkflowStatus),
            selected_workflow: None,
            diagnostics_scope: None,
            function_id: None,
            function_args: None,
            reason: None,
        };

        return route_selector_action(
            &pseudo_request,
            &status_result,
            RouteContext {
                status_input: &status_input,
                active_conversation_runs,
                functions,
                run_store: &run_store,
                orchestrator: &OrchestratorConfig {
                    id: "status_only".to_string(),
                    selector_agent: "none".to_string(),
                    default_workflow: "none".to_string(),
                    selection_max_retries: 1,
                    agents: BTreeMap::new(),
                    workflows: Vec::new(),
                    workflow_orchestration: None,
                },
                source_message_id: Some(&inbound.message_id),
                now,
            },
        );
    }

    let orchestrator_id = resolve_orchestrator_id(settings, inbound)?;
    let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
    let workspace_context = resolve_workspace_access_context(settings, &orchestrator_id)?;
    let requested_paths =
        collect_orchestrator_requested_paths(&workspace_context, settings, &orchestrator)?;
    if let Err(err) = enforce_workspace_access(&workspace_context, &requested_paths) {
        append_security_log(
            state_root,
            &format!(
                "workspace access denied for orchestrator `{orchestrator_id}` message `{}`: {err}",
                inbound.message_id
            ),
        );
        return Err(err);
    }

    let request = SelectorRequest {
        selector_id: format!("sel-{}", inbound.message_id),
        channel_profile_id: inbound
            .channel_profile_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        message_id: inbound.message_id.clone(),
        conversation_id: inbound.conversation_id.clone(),
        user_message: inbound.message.clone(),
        available_workflows: orchestrator
            .workflows
            .iter()
            .map(|w| w.id.clone())
            .collect(),
        default_workflow: orchestrator.default_workflow.clone(),
        available_functions: functions.available_function_ids(),
    };

    let artifact_store = SelectorArtifactStore::new(state_root);
    artifact_store.persist_message_snapshot(inbound)?;
    artifact_store.persist_selector_request(&request)?;
    let _ = artifact_store.move_request_to_processing(&request.selector_id)?;

    let selection = resolve_selector_with_retries(&orchestrator, &request, |attempt| {
        next_selector_attempt(attempt, &request, &orchestrator)
    });
    artifact_store.persist_selector_result(&selection.result)?;
    artifact_store.persist_selector_log(
        &request.selector_id,
        selection
            .result
            .reason
            .as_deref()
            .unwrap_or("selector completed"),
    )?;

    let status_input = StatusResolutionInput {
        explicit_run_id: None,
        inbound_workflow_run_id: inbound.workflow_run_id.clone(),
        channel_profile_id: inbound.channel_profile_id.clone(),
        conversation_id: inbound.conversation_id.clone(),
    };
    route_selector_action(
        &request,
        &selection.result,
        RouteContext {
            status_input: &status_input,
            active_conversation_runs,
            functions,
            run_store: &run_store,
            orchestrator: &orchestrator,
            source_message_id: Some(&inbound.message_id),
            now,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

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

    #[test]
    fn workspace_access_context_and_enforcement_allow_private_and_granted_shared_only() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspace_path: /tmp/workspace
shared_workspaces:
  docs: /tmp/shared/docs
  finance: /tmp/shared/finance
orchestrators:
  alpha:
    shared_access: [docs]
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("settings");

        let context = resolve_workspace_access_context(&settings, "alpha").expect("context");
        assert_eq!(context.shared_workspace_roots.len(), 1);
        assert!(context.shared_workspace_roots.contains_key("docs"));

        enforce_workspace_access(
            &context,
            &[
                PathBuf::from("/tmp/workspace/orchestrators/alpha/agents/worker"),
                PathBuf::from("/tmp/shared/docs/project/readme.md"),
            ],
        )
        .expect("allowed paths");

        let err = enforce_workspace_access(
            &context,
            &[PathBuf::from("/tmp/shared/finance/budget.xlsx")],
        )
        .expect_err("must deny ungranted shared path");
        assert!(err.to_string().contains("workspace access denied"));
    }

    #[test]
    fn output_path_resolution_interpolates_and_blocks_traversal() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join(".direclaw");

        let step = WorkflowStepConfig {
            id: "plan".to_string(),
            step_type: "agent_task".to_string(),
            agent: "worker".to_string(),
            prompt: "prompt".to_string(),
            next: None,
            on_approve: None,
            on_reject: None,
            outputs: Some(vec!["artifact".to_string()]),
            output_files: Some(BTreeMap::from_iter([(
                "artifact".to_string(),
                "artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}.md"
                    .to_string(),
            )])),
            limits: None,
        };

        let resolved =
            resolve_step_output_paths(&state_root, "run-123", &step, 2).expect("resolved paths");
        let artifact = resolved.get("artifact").expect("artifact path");
        assert!(artifact
            .starts_with(state_root.join("workflows/runs/run-123/steps/plan/attempts/2/outputs")));
        assert!(artifact
            .display()
            .to_string()
            .ends_with("artifacts/run-123/plan-2.md"));

        let bad_step = WorkflowStepConfig {
            output_files: Some(BTreeMap::from_iter([(
                "artifact".to_string(),
                "../escape.md".to_string(),
            )])),
            ..step
        };
        let err =
            resolve_step_output_paths(&state_root, "run-123", &bad_step, 1).expect_err("blocked");
        assert!(err.to_string().contains("output path validation failed"));
    }
}
