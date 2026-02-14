use crate::cli;
use crate::config::{
    load_orchestrator_config, AgentConfig, ConfigError, OrchestratorConfig, Settings,
    WorkflowConfig, WorkflowStepConfig,
};
use crate::provider::{
    consume_reset_flag, run_provider, write_file_backed_prompt, InvocationLog, ProviderError,
    ProviderKind, ProviderRequest, RunnerBinaries,
};
use crate::queue::IncomingMessage;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

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
    #[error("unknown function argument `{arg}` for `{function_id}`")]
    UnknownFunctionArg { function_id: String, arg: String },
    #[error("invalid argument type for `{function_id}.{arg}`; expected {expected}")]
    InvalidFunctionArgType {
        function_id: String,
        arg: String,
        expected: String,
    },
    #[error("workflow run `{run_id}` not found")]
    UnknownRunId { run_id: String },
    #[error("workflow run state transition `{from}` -> `{to}` is invalid")]
    InvalidRunTransition { from: RunState, to: RunState },
    #[error("workflow result envelope parse failed: {0}")]
    WorkflowEnvelope(String),
    #[error("workflow review decision must be `approve` or `reject`, got `{0}`")]
    InvalidReviewDecision(String),
    #[error("step prompt render failed for step `{step_id}`: {reason}")]
    StepPromptRender { step_id: String, reason: String },
    #[error("step execution failed for step `{step_id}`: {reason}")]
    StepExecution { step_id: String, reason: String },
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
    #[error("step `{step_id}` output contract validation failed: {reason}")]
    OutputContractValidation { step_id: String, reason: String },
    #[error("step `{step_id}` transition validation failed: {reason}")]
    TransitionValidation { step_id: String, reason: String },
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

fn elapsed_now(base_now: i64, started_at: Instant) -> i64 {
    base_now.saturating_add(started_at.elapsed().as_secs() as i64)
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
#[serde(rename_all = "snake_case")]
pub enum FunctionArgType {
    String,
    Boolean,
    Integer,
    Object,
}

impl FunctionArgType {
    fn matches(&self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Boolean => value.is_boolean(),
            Self::Integer => value.is_i64() || value.is_u64(),
            Self::Object => value.is_object(),
        }
    }
}

impl std::fmt::Display for FunctionArgType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String => write!(f, "string"),
            Self::Boolean => write!(f, "boolean"),
            Self::Integer => write!(f, "integer"),
            Self::Object => write!(f, "object"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionArgSchema {
    #[serde(rename = "type")]
    pub arg_type: FunctionArgType,
    pub required: bool,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionSchema {
    pub function_id: String,
    pub description: String,
    #[serde(default)]
    pub args: BTreeMap<String, FunctionArgSchema>,
    pub read_only: bool,
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
    #[serde(default)]
    pub available_function_schemas: Vec<FunctionSchema>,
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
                    if let Some(schema) = request
                        .available_function_schemas
                        .iter()
                        .find(|schema| schema.function_id == *function_id)
                    {
                        let args = result.function_args.as_ref().expect("checked above");
                        for key in args.keys() {
                            if !schema.args.contains_key(key) {
                                return Err(OrchestratorError::SelectorValidation(format!(
                                    "command_invoke has unknown argument `{key}` for function `{function_id}`"
                                )));
                            }
                        }
                        for (arg, arg_schema) in &schema.args {
                            match args.get(arg) {
                                Some(value) if arg_schema.arg_type.matches(value) => {}
                                Some(_) => {
                                    return Err(OrchestratorError::SelectorValidation(format!(
                                        "command_invoke argument `{arg}` for function `{function_id}` must be {}",
                                        arg_schema.arg_type
                                    )))
                                }
                                None if arg_schema.required => {
                                    return Err(OrchestratorError::SelectorValidation(format!(
                                        "command_invoke missing required argument `{arg}` for function `{function_id}`"
                                    )))
                                }
                                None => {}
                            }
                        }
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
                | (RunState::Queued, RunState::Failed)
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
    pub inputs: Map<String, Value>,
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
    #[serde(default)]
    pub terminal_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSnapshot {
    pub run_id: String,
    pub workflow_id: String,
    pub state: RunState,
    #[serde(default)]
    pub input_count: usize,
    #[serde(default)]
    pub input_keys: Vec<String>,
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
    pub output_files: BTreeMap<String, String>,
    #[serde(default)]
    pub next_step_id: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub output_validation_errors: BTreeMap<String, String>,
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
        self.create_run_with_inputs(run_id, workflow_id, Map::new(), now)
    }

    pub fn create_run_with_inputs(
        &self,
        run_id: impl Into<String>,
        workflow_id: impl Into<String>,
        inputs: Map<String, Value>,
        now: i64,
    ) -> Result<WorkflowRunRecord, OrchestratorError> {
        self.create_run_with_metadata(
            run_id,
            workflow_id,
            SelectorStartedRunMetadata::default(),
            inputs,
            now,
        )
    }

    pub fn create_run_with_metadata(
        &self,
        run_id: impl Into<String>,
        workflow_id: impl Into<String>,
        metadata: SelectorStartedRunMetadata,
        inputs: Map<String, Value>,
        now: i64,
    ) -> Result<WorkflowRunRecord, OrchestratorError> {
        let input_keys = sorted_input_keys(&inputs);
        let run = WorkflowRunRecord {
            run_id: run_id.into(),
            workflow_id: workflow_id.into(),
            state: RunState::Queued,
            inputs,
            current_step_id: None,
            current_attempt: None,
            started_at: now,
            updated_at: now,
            total_iterations: 0,
            source_message_id: metadata.source_message_id,
            selector_id: metadata.selector_id,
            selected_workflow: metadata.selected_workflow,
            status_conversation_id: metadata.status_conversation_id,
            terminal_reason: None,
        };
        self.persist_run(&run)?;
        self.persist_progress(&ProgressSnapshot {
            run_id: run.run_id.clone(),
            workflow_id: run.workflow_id.clone(),
            state: run.state.clone(),
            input_count: run.inputs.len(),
            input_keys,
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
        let summary = summary.into();
        let next_expected_action = next_expected_action.into();
        if !run.state.clone().can_transition_to(next.clone()) {
            return Err(OrchestratorError::InvalidRunTransition {
                from: run.state.clone(),
                to: next,
            });
        }
        run.state = next;
        run.updated_at = now;
        if run.state.clone().is_terminal() {
            run.terminal_reason = Some(summary.clone());
        } else {
            run.terminal_reason = None;
        }
        self.persist_run(run)?;
        self.persist_progress(&ProgressSnapshot {
            run_id: run.run_id.clone(),
            workflow_id: run.workflow_id.clone(),
            state: run.state.clone(),
            input_count: run.inputs.len(),
            input_keys: sorted_input_keys(&run.inputs),
            current_step_id: run.current_step_id.clone(),
            current_attempt: run.current_attempt,
            started_at: run.started_at,
            updated_at: now,
            last_progress_at: now,
            summary,
            pending_human_input,
            next_expected_action,
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
            input_count: run.inputs.len(),
            input_keys: sorted_input_keys(&run.inputs),
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
            input_count: run.inputs.len(),
            input_keys: sorted_input_keys(&run.inputs),
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
            input_count: run.inputs.len(),
            input_keys: sorted_input_keys(&run.inputs),
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

    fn engine_log_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("engine.log")
    }

    pub fn append_engine_log(
        &self,
        run_id: &str,
        now: i64,
        message: impl AsRef<str>,
    ) -> Result<(), OrchestratorError> {
        let path = self.engine_log_path(run_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| io_error(&path, e))?;
        writeln!(file, "ts={now} {}", message.as_ref()).map_err(|e| io_error(&path, e))
    }

    pub fn load_step_attempt(
        &self,
        run_id: &str,
        step_id: &str,
        attempt: u32,
    ) -> Result<StepAttemptRecord, OrchestratorError> {
        let path = self
            .run_dir(run_id)
            .join("steps")
            .join(step_id)
            .join("attempts")
            .join(attempt.to_string())
            .join("result.json");
        let raw = fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
        serde_json::from_str(&raw).map_err(|e| json_error(&path, e))
    }

    pub fn checkpoint(
        &self,
        run: &mut WorkflowRunRecord,
        now: i64,
        summary: impl Into<String>,
        pending_human_input: bool,
        next_expected_action: impl Into<String>,
    ) -> Result<(), OrchestratorError> {
        run.updated_at = now;
        self.persist_run(run)?;
        self.persist_progress(&ProgressSnapshot {
            run_id: run.run_id.clone(),
            workflow_id: run.workflow_id.clone(),
            state: run.state.clone(),
            input_count: run.inputs.len(),
            input_keys: sorted_input_keys(&run.inputs),
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
}

fn sorted_input_keys(inputs: &Map<String, Value>) -> Vec<String> {
    let mut keys = inputs.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}

#[derive(Debug, Clone)]
pub struct WorkflowEngine {
    run_store: WorkflowRunStore,
    orchestrator: OrchestratorConfig,
    runner_binaries: RunnerBinaries,
    workspace_access_context: Option<WorkspaceAccessContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NextStepPointer {
    step_id: String,
    attempt: u32,
}

impl WorkflowEngine {
    pub fn new(run_store: WorkflowRunStore, orchestrator: OrchestratorConfig) -> Self {
        Self {
            run_store,
            orchestrator,
            runner_binaries: resolve_runner_binaries(),
            workspace_access_context: None,
        }
    }

    pub fn with_runner_binaries(mut self, runner_binaries: RunnerBinaries) -> Self {
        self.runner_binaries = runner_binaries;
        self
    }

    pub fn with_workspace_access_context(
        mut self,
        workspace_access_context: WorkspaceAccessContext,
    ) -> Self {
        self.workspace_access_context = Some(workspace_access_context);
        self
    }

    pub fn start(&self, run_id: &str, now: i64) -> Result<WorkflowRunRecord, OrchestratorError> {
        let mut run = self.run_store.load_run(run_id)?;
        if run.state == RunState::Queued {
            let workflow_id = run.workflow_id.clone();
            self.run_store.transition_state(
                &mut run,
                RunState::Running,
                now,
                format!("workflow {workflow_id} started"),
                false,
                "execute next step",
            )?;
        }
        self.run_until_non_running(&mut run, now.saturating_add(1))?;
        self.run_store.load_run(run_id)
    }

    pub fn resume(&self, run_id: &str, now: i64) -> Result<WorkflowRunRecord, OrchestratorError> {
        let mut run = self.run_store.load_run(run_id)?;
        if run.state.clone().is_terminal() {
            return Ok(run);
        }
        if run.state == RunState::Queued || run.state == RunState::Waiting {
            let workflow_id = run.workflow_id.clone();
            self.run_store.transition_state(
                &mut run,
                RunState::Running,
                now,
                format!("workflow {workflow_id} resumed"),
                false,
                "execute next step",
            )?;
        }
        self.run_until_non_running(&mut run, now.saturating_add(1))?;
        self.run_store.load_run(run_id)
    }

    fn run_until_non_running(
        &self,
        run: &mut WorkflowRunRecord,
        start_now: i64,
    ) -> Result<(), OrchestratorError> {
        // Guard against accidental infinite loops in malformed workflows.
        let max_cycles = 10_000u32;
        let mut cycles = 0u32;
        let run_clock_started = Instant::now();

        while run.state == RunState::Running {
            if cycles >= max_cycles {
                return Err(OrchestratorError::SelectorValidation(
                    "workflow engine exceeded maximum execution cycles".to_string(),
                ));
            }
            let previous_iterations = run.total_iterations;
            let now = elapsed_now(start_now, run_clock_started);
            self.execute_or_fail(run, now)?;
            *run = self.run_store.load_run(&run.run_id)?;
            if run.state == RunState::Running && run.total_iterations == previous_iterations {
                return Err(OrchestratorError::SelectorValidation(
                    "workflow engine made no progress while running".to_string(),
                ));
            }
            cycles = cycles.saturating_add(1);
        }

        Ok(())
    }

    pub fn execute_next(
        &self,
        run: &mut WorkflowRunRecord,
        now: i64,
    ) -> Result<(), OrchestratorError> {
        let workflow = self.workflow_for_run(run)?;
        let Some(pointer) = self.resolve_next_step_pointer(run, workflow)? else {
            run.current_step_id = None;
            run.current_attempt = None;
            return self.run_store.transition_state(
                run,
                RunState::Succeeded,
                now,
                "workflow completed",
                false,
                "none",
            );
        };

        let step = workflow
            .steps
            .iter()
            .find(|step| step.id == pointer.step_id)
            .ok_or_else(|| {
                OrchestratorError::SelectorValidation(format!(
                    "workflow `{}` missing step `{}`",
                    workflow.id, pointer.step_id
                ))
            })?;

        let limits = resolve_execution_safety_limits(&self.orchestrator, workflow, step);
        let mut attempt = pointer.attempt;
        let step_clock_started = Instant::now();

        loop {
            let attempt_started_at = elapsed_now(now, step_clock_started);
            self.run_store.append_engine_log(
                &run.run_id,
                attempt_started_at,
                format!(
                    "run_id={} decision=execute_next step_id={} attempt={} state={}",
                    run.run_id, step.id, attempt, run.state
                ),
            )?;
            self.run_store
                .mark_step_attempt_started(run, &step.id, attempt, attempt_started_at)?;
            enforce_execution_safety(run, limits, attempt_started_at, attempt_started_at, attempt)?;

            match self.execute_step_attempt(run, workflow, step, attempt, attempt_started_at) {
                Ok(evaluation) => {
                    let attempt_ended_at = elapsed_now(now, step_clock_started);
                    self.run_store.persist_step_attempt(&StepAttemptRecord {
                        run_id: run.run_id.clone(),
                        step_id: step.id.clone(),
                        attempt,
                        started_at: attempt_started_at,
                        ended_at: attempt_ended_at,
                        state: "succeeded".to_string(),
                        outputs: evaluation.outputs.clone(),
                        output_files: evaluation.output_files.clone(),
                        next_step_id: evaluation.next_step_id.clone(),
                        error: None,
                        output_validation_errors: BTreeMap::new(),
                    })?;
                    *run = self.run_store.load_run(&run.run_id)?;

                    self.run_store.append_engine_log(
                        &run.run_id,
                        attempt_ended_at,
                        format!(
                            "run_id={} step_id={} attempt={} transition=succeeded next={}",
                            run.run_id,
                            step.id,
                            attempt,
                            evaluation
                                .next_step_id
                                .clone()
                                .unwrap_or_else(|| "terminal".to_string())
                        ),
                    )?;
                    enforce_execution_safety(
                        run,
                        limits,
                        attempt_ended_at,
                        attempt_started_at,
                        attempt,
                    )?;

                    if let Some(next) = evaluation.next_step_id {
                        run.current_step_id = Some(next.clone());
                        run.current_attempt = None;
                        self.run_store.checkpoint(
                            run,
                            attempt_ended_at,
                            format!(
                                "step {} attempt {} finished; next {}",
                                step.id, attempt, next
                            ),
                            false,
                            format!("execute step {next}"),
                        )?;
                    } else {
                        run.current_step_id = None;
                        run.current_attempt = None;
                        self.run_store.transition_state(
                            run,
                            RunState::Succeeded,
                            attempt_ended_at,
                            format!("step {} attempt {} finished", step.id, attempt),
                            false,
                            "none",
                        )?;
                    }
                    return Ok(());
                }
                Err(err) => {
                    let attempt_ended_at = elapsed_now(now, step_clock_started);
                    let retryable = is_retryable_step_error(&err);
                    let can_retry = retryable && attempt <= limits.max_retries;
                    let output_validation_errors = output_validation_errors_for(&err);
                    self.run_store.persist_step_attempt(&StepAttemptRecord {
                        run_id: run.run_id.clone(),
                        step_id: step.id.clone(),
                        attempt,
                        started_at: attempt_started_at,
                        ended_at: attempt_ended_at,
                        state: if can_retry {
                            "failed_retryable".to_string()
                        } else {
                            "failed".to_string()
                        },
                        outputs: Map::new(),
                        output_files: BTreeMap::new(),
                        next_step_id: None,
                        error: Some(err.to_string()),
                        output_validation_errors,
                    })?;
                    *run = self.run_store.load_run(&run.run_id)?;
                    self.run_store.append_engine_log(
                        &run.run_id,
                        attempt_ended_at,
                        format!(
                            "run_id={} step_id={} attempt={} transition=failed retryable={} error={}",
                            run.run_id, step.id, attempt, can_retry, err
                        ),
                    )?;
                    enforce_execution_safety(
                        run,
                        limits,
                        attempt_ended_at,
                        attempt_started_at,
                        attempt,
                    )?;
                    if can_retry {
                        attempt = attempt.saturating_add(1);
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    fn execute_or_fail(
        &self,
        run: &mut WorkflowRunRecord,
        now: i64,
    ) -> Result<(), OrchestratorError> {
        match self.execute_next(run, now) {
            Ok(()) => Ok(()),
            Err(err) => {
                let reason = format!("engine start failed: {err}");
                self.run_store.append_engine_log(
                    &run.run_id,
                    now,
                    format!("run_id={} transition=failed reason={reason}", run.run_id),
                )?;
                if !run.state.clone().is_terminal() {
                    self.run_store.transition_state(
                        run,
                        RunState::Failed,
                        now,
                        reason,
                        false,
                        "inspect workflow run artifacts",
                    )?;
                }
                Err(err)
            }
        }
    }

    fn workflow_for_run<'a>(
        &'a self,
        run: &WorkflowRunRecord,
    ) -> Result<&'a WorkflowConfig, OrchestratorError> {
        self.orchestrator
            .workflows
            .iter()
            .find(|workflow| workflow.id == run.workflow_id)
            .ok_or_else(|| {
                OrchestratorError::SelectorValidation(format!(
                    "workflow `{}` is not declared in orchestrator",
                    run.workflow_id
                ))
            })
    }

    fn resolve_next_step_pointer(
        &self,
        run: &WorkflowRunRecord,
        workflow: &WorkflowConfig,
    ) -> Result<Option<NextStepPointer>, OrchestratorError> {
        let Some(current_step) = run.current_step_id.as_ref() else {
            return Ok(workflow.steps.first().map(|step| NextStepPointer {
                step_id: step.id.clone(),
                attempt: 1,
            }));
        };

        let Some(current_attempt) = run.current_attempt else {
            return Ok(Some(NextStepPointer {
                step_id: current_step.clone(),
                attempt: 1,
            }));
        };

        let persisted =
            self.run_store
                .load_step_attempt(&run.run_id, current_step, current_attempt);
        match persisted {
            Ok(attempt_record) => {
                if let Some(next) = attempt_record.next_step_id {
                    Ok(Some(NextStepPointer {
                        step_id: next,
                        attempt: 1,
                    }))
                } else {
                    Ok(None)
                }
            }
            Err(OrchestratorError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(Some(NextStepPointer {
                    step_id: current_step.clone(),
                    attempt: current_attempt,
                }))
            }
            Err(other) => Err(other),
        }
    }

    fn execute_step_attempt(
        &self,
        run: &WorkflowRunRecord,
        workflow: &WorkflowConfig,
        step: &WorkflowStepConfig,
        attempt: u32,
        now: i64,
    ) -> Result<StepEvaluation, OrchestratorError> {
        let run_workspace = if let Some(context) = self.workspace_access_context.as_ref() {
            context
                .private_workspace_root
                .join("workflows/runs")
                .join(&run.run_id)
                .join("workspace")
        } else {
            self.run_store
                .state_root()
                .join("workflows/runs")
                .join(&run.run_id)
                .join("workspace")
        };

        let agent = self.orchestrator.agents.get(&step.agent).ok_or_else(|| {
            OrchestratorError::StepExecution {
                step_id: step.id.clone(),
                reason: format!("step references unknown agent `{}`", step.agent),
            }
        })?;
        let agent_workspace = resolve_agent_workspace_root(
            self.workspace_access_context
                .as_ref()
                .map(|ctx| ctx.private_workspace_root.as_path())
                .unwrap_or_else(|| self.run_store.state_root()),
            &step.agent,
            agent,
        );

        if let Some(context) = self.workspace_access_context.as_ref() {
            if let Err(err) =
                enforce_workspace_access(context, &[agent_workspace.clone(), run_workspace.clone()])
            {
                append_security_log(
                    self.run_store.state_root(),
                    &format!(
                        "workspace access denied for run `{}` step `{}`: {}",
                        run.run_id, step.id, err
                    ),
                );
                return Err(err);
            }
        }
        fs::create_dir_all(&run_workspace).map_err(|err| io_error(&run_workspace, err))?;
        fs::create_dir_all(&agent_workspace).map_err(|err| io_error(&agent_workspace, err))?;

        let step_outputs =
            load_latest_step_outputs(self.run_store.state_root(), &run.run_id, workflow, &step.id)?;
        let output_paths = match resolve_step_output_paths(
            self.run_store.state_root(),
            &run.run_id,
            step,
            attempt,
        ) {
            Ok(paths) => paths,
            Err(err @ OrchestratorError::OutputPathValidation { .. }) => {
                append_security_log(
                    self.run_store.state_root(),
                    &format!(
                        "output path validation denied for run `{}` step `{}` attempt `{}`: {}",
                        run.run_id, step.id, attempt, err
                    ),
                );
                return Err(err);
            }
            Err(err) => return Err(err),
        };
        let rendered = render_step_prompt(
            run,
            workflow,
            step,
            attempt,
            &run_workspace,
            &output_paths,
            &step_outputs,
        )?;

        let attempt_dir = self
            .run_store
            .state_root()
            .join("workflows/runs")
            .join(&run.run_id)
            .join("steps")
            .join(&step.id)
            .join("attempts")
            .join(attempt.to_string());
        fs::create_dir_all(&attempt_dir).map_err(|err| io_error(&attempt_dir, err))?;

        let artifacts = write_file_backed_prompt(
            &attempt_dir,
            &format!("{}-{}-{attempt}", run.run_id, step.id),
            &rendered.prompt,
            &rendered.context,
        )
        .map_err(|err| OrchestratorError::StepExecution {
            step_id: step.id.clone(),
            reason: err.to_string(),
        })?;

        let reset_flag = agent_workspace.join("reset_flag");
        let reset_resolution =
            consume_reset_flag(&reset_flag).map_err(|err| OrchestratorError::StepExecution {
                step_id: step.id.clone(),
                reason: err.to_string(),
            })?;

        let provider_kind = ProviderKind::try_from(agent.provider.as_str()).map_err(|err| {
            OrchestratorError::StepExecution {
                step_id: step.id.clone(),
                reason: err.to_string(),
            }
        })?;
        let provider_request = ProviderRequest {
            agent_id: step.agent.clone(),
            provider: provider_kind,
            model: agent.model.clone(),
            cwd: agent_workspace.clone(),
            message: provider_instruction_message(&artifacts),
            prompt_artifacts: artifacts.clone(),
            timeout: Duration::from_secs(
                resolve_execution_safety_limits(&self.orchestrator, workflow, step)
                    .step_timeout_seconds,
            ),
            reset_requested: reset_resolution.reset_requested,
            fresh_on_failure: false,
            env_overrides: BTreeMap::new(),
        };

        let step_timeout_seconds =
            resolve_execution_safety_limits(&self.orchestrator, workflow, step)
                .step_timeout_seconds;
        let provider_output =
            run_provider(&provider_request, &self.runner_binaries).map_err(|err| {
                if let Some(log) = provider_error_log(&err) {
                    let _ = persist_provider_invocation_log(&attempt_dir, log);
                }
                match err {
                    ProviderError::Timeout { .. } => OrchestratorError::StepTimeout {
                        step_timeout_seconds,
                    },
                    _ => OrchestratorError::StepExecution {
                        step_id: step.id.clone(),
                        reason: err.to_string(),
                    },
                }
            })?;

        persist_provider_invocation_log(&attempt_dir, &provider_output.log)
            .map_err(|err| io_error(&attempt_dir, err))?;

        let mut evaluation = evaluate_step_result(workflow, step, &provider_output.message)?;
        evaluation.output_files = materialize_output_files(
            self.run_store.state_root(),
            &run.run_id,
            step,
            attempt,
            &evaluation.outputs,
        )?;
        self.run_store.append_engine_log(
            &run.run_id,
            now,
            format!(
                "run_id={} step_id={} attempt={} provider={} model={} cwd={}",
                run.run_id,
                step.id,
                attempt,
                provider_output.log.provider,
                provider_output.log.model,
                provider_output.log.working_directory.display(),
            ),
        )?;
        Ok(evaluation)
    }
}

fn provider_instruction_message(artifacts: &crate::provider::PromptArtifacts) -> String {
    let context_paths = artifacts
        .context_files
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Read prompt file at {} and context file(s) at {}. Execute exactly as instructed in those files.",
        artifacts.prompt_file.display(),
        context_paths
    )
}

fn resolve_runner_binaries() -> RunnerBinaries {
    RunnerBinaries {
        anthropic: std::env::var("DIRECLAW_PROVIDER_BIN_ANTHROPIC")
            .unwrap_or_else(|_| "claude".to_string()),
        openai: std::env::var("DIRECLAW_PROVIDER_BIN_OPENAI")
            .unwrap_or_else(|_| "codex".to_string()),
    }
}

fn provider_error_log(error: &ProviderError) -> Option<&InvocationLog> {
    match error {
        ProviderError::MissingBinary { log, .. } => Some(log),
        ProviderError::NonZeroExit { log, .. } => Some(log),
        ProviderError::Timeout { log, .. } => Some(log),
        ProviderError::ParseFailure { log, .. } => log.as_deref(),
        ProviderError::UnknownProvider(_)
        | ProviderError::UnsupportedAnthropicModel(_)
        | ProviderError::Io { .. } => None,
    }
}

fn persist_provider_invocation_log(path_root: &Path, log: &InvocationLog) -> std::io::Result<()> {
    let path = path_root.join("provider_invocation.json");
    let payload = Value::Object(Map::from_iter([
        ("agentId".to_string(), Value::String(log.agent_id.clone())),
        (
            "provider".to_string(),
            Value::String(log.provider.to_string()),
        ),
        ("model".to_string(), Value::String(log.model.clone())),
        (
            "commandForm".to_string(),
            Value::String(log.command_form.clone()),
        ),
        (
            "workingDirectory".to_string(),
            Value::String(log.working_directory.display().to_string()),
        ),
        (
            "promptFile".to_string(),
            Value::String(log.prompt_file.display().to_string()),
        ),
        (
            "contextFiles".to_string(),
            Value::Array(
                log.context_files
                    .iter()
                    .map(|path| Value::String(path.display().to_string()))
                    .collect(),
            ),
        ),
        (
            "exitCode".to_string(),
            match log.exit_code {
                Some(value) => Value::from(value),
                None => Value::Null,
            },
        ),
        ("timedOut".to_string(), Value::Bool(log.timed_out)),
    ]));
    let body = serde_json::to_vec_pretty(&payload).map_err(std::io::Error::other)?;
    fs::write(path, body)
}

fn is_retryable_step_error(error: &OrchestratorError) -> bool {
    matches!(
        error,
        OrchestratorError::StepExecution { .. }
            | OrchestratorError::WorkflowEnvelope(_)
            | OrchestratorError::InvalidReviewDecision(_)
            | OrchestratorError::OutputContractValidation { .. }
    )
}

fn load_latest_step_outputs(
    state_root: &Path,
    run_id: &str,
    workflow: &WorkflowConfig,
    current_step_id: &str,
) -> Result<BTreeMap<String, Map<String, Value>>, OrchestratorError> {
    let mut outputs = BTreeMap::new();
    for step in &workflow.steps {
        if step.id == current_step_id {
            continue;
        }
        let attempts_root = state_root
            .join("workflows/runs")
            .join(run_id)
            .join("steps")
            .join(&step.id)
            .join("attempts");
        if !attempts_root.exists() {
            continue;
        }

        let mut latest_attempt: Option<(u32, StepAttemptRecord)> = None;
        let entries = fs::read_dir(&attempts_root).map_err(|err| io_error(&attempts_root, err))?;
        for entry in entries {
            let entry = entry.map_err(|err| io_error(&attempts_root, err))?;
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            let Ok(attempt_num) = name.parse::<u32>() else {
                continue;
            };
            let result_path = entry.path().join("result.json");
            if !result_path.is_file() {
                continue;
            }
            let raw =
                fs::read_to_string(&result_path).map_err(|err| io_error(&result_path, err))?;
            let attempt: StepAttemptRecord =
                serde_json::from_str(&raw).map_err(|err| json_error(&result_path, err))?;
            if attempt.state != "succeeded" {
                continue;
            }
            match latest_attempt {
                Some((current, _)) if current >= attempt_num => {}
                _ => latest_attempt = Some((attempt_num, attempt)),
            }
        }

        if let Some((_, attempt)) = latest_attempt {
            outputs.insert(step.id.clone(), attempt.outputs);
        }
    }
    Ok(outputs)
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
    if output[start + open_tag.len()..].contains(open_tag) {
        return Err(OrchestratorError::WorkflowEnvelope(
            "multiple [workflow_result] tags are not allowed".to_string(),
        ));
    }
    if output[end + close_tag.len()..].contains(close_tag) {
        return Err(OrchestratorError::WorkflowEnvelope(
            "multiple [/workflow_result] tags are not allowed".to_string(),
        ));
    }
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
struct OutputContractKey {
    name: String,
    required: bool,
}

fn parse_output_contract_key(raw: &str) -> Result<OutputContractKey, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("output key must be non-empty".to_string());
    }
    let (name, required) = if let Some(optional) = trimmed.strip_suffix('?') {
        (optional.trim(), false)
    } else {
        (trimmed, true)
    };
    if name.is_empty() {
        return Err("output key must be non-empty".to_string());
    }
    if name.contains('?') {
        return Err("output key may only contain optional marker as trailing `?`".to_string());
    }
    Ok(OutputContractKey {
        name: name.to_string(),
        required,
    })
}

fn parse_output_contract(
    step: &WorkflowStepConfig,
) -> Result<Vec<OutputContractKey>, OrchestratorError> {
    let Some(configured) = step.outputs.as_ref() else {
        return Ok(Vec::new());
    };
    let mut keys = Vec::with_capacity(configured.len());
    for raw in configured {
        let key = parse_output_contract_key(raw).map_err(|reason| {
            OrchestratorError::OutputContractValidation {
                step_id: step.id.clone(),
                reason: format!("invalid output declaration `{raw}`: {reason}"),
            }
        })?;
        keys.push(key);
    }
    Ok(keys)
}

fn validate_outputs_contract(
    step: &WorkflowStepConfig,
    outputs: &Map<String, Value>,
) -> Result<(), OrchestratorError> {
    let contract = parse_output_contract(step)?;
    if contract.is_empty() {
        return Ok(());
    }

    let mut missing_required = Vec::new();
    for key in contract.into_iter().filter(|key| key.required) {
        if !outputs.contains_key(&key.name) {
            missing_required.push(key.name);
        }
    }
    if missing_required.is_empty() {
        return Ok(());
    }

    missing_required.sort();
    let details = missing_required
        .iter()
        .map(|key| format!("{key}=missing"))
        .collect::<Vec<_>>()
        .join(", ");
    Err(OrchestratorError::OutputContractValidation {
        step_id: step.id.clone(),
        reason: format!("missing required output keys: {details}"),
    })
}

fn output_validation_errors_for(error: &OrchestratorError) -> BTreeMap<String, String> {
    match error {
        OrchestratorError::OutputContractValidation { reason, .. } => reason
            .trim()
            .strip_prefix("missing required output keys:")
            .unwrap_or(reason.as_str())
            .split(',')
            .filter_map(|entry| {
                let mut parts = entry.trim().splitn(2, '=');
                let key = parts.next()?.trim();
                let detail = parts.next()?.trim();
                if key.is_empty() || detail.is_empty() {
                    return None;
                }
                Some((key.to_string(), detail.to_string()))
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn validate_transition_target(
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
    target: Option<String>,
    reason: &str,
) -> Result<Option<String>, OrchestratorError> {
    let Some(target) = target else {
        return Ok(None);
    };
    if workflow
        .steps
        .iter()
        .any(|candidate| candidate.id == target)
    {
        return Ok(Some(target));
    }
    Err(OrchestratorError::TransitionValidation {
        step_id: step.id.clone(),
        reason: format!("{reason} targets unknown step `{target}`"),
    })
}

fn materialize_output_files(
    state_root: &Path,
    run_id: &str,
    step: &WorkflowStepConfig,
    attempt: u32,
    outputs: &Map<String, Value>,
) -> Result<BTreeMap<String, String>, OrchestratorError> {
    let output_paths = resolve_step_output_paths(state_root, run_id, step, attempt)?;
    if output_paths.is_empty() {
        return Ok(BTreeMap::new());
    }

    let contract = parse_output_contract(step)?;
    let mut path_by_key = BTreeMap::new();
    for key in contract {
        let Some(value) = outputs.get(&key.name) else {
            continue;
        };
        let Some(path) = output_paths.get(&key.name) else {
            return Err(OrchestratorError::OutputContractValidation {
                step_id: step.id.clone(),
                reason: format!("missing output_files mapping for key `{}`", key.name),
            });
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let content = if let Some(text) = value.as_str() {
            text.as_bytes().to_vec()
        } else {
            serde_json::to_vec_pretty(value).map_err(|e| OrchestratorError::StepExecution {
                step_id: step.id.clone(),
                reason: format!("failed to serialize output key `{}`: {e}", key.name),
            })?
        };
        fs::write(path, content).map_err(|e| io_error(path, e))?;
        path_by_key.insert(key.name, path.display().to_string());
    }

    Ok(path_by_key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEvaluation {
    pub outputs: Map<String, Value>,
    pub output_files: BTreeMap<String, String>,
    pub next_step_id: Option<String>,
}

pub fn evaluate_step_result(
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
    raw_output: &str,
) -> Result<StepEvaluation, OrchestratorError> {
    let parsed = parse_workflow_result_envelope(raw_output)?;
    validate_outputs_contract(step, &parsed)?;
    if step.step_type == "agent_review" {
        let approve = parse_review_decision(&parsed)?;
        let next = if approve {
            step.on_approve.clone()
        } else {
            step.on_reject.clone()
        };
        if next.is_none() {
            return Err(OrchestratorError::TransitionValidation {
                step_id: step.id.clone(),
                reason: if approve {
                    "decision `approve` requires `on_approve` transition target".to_string()
                } else {
                    "decision `reject` requires `on_reject` transition target".to_string()
                },
            });
        }
        let next = validate_transition_target(workflow, step, next, "review transition")?;
        return Ok(StepEvaluation {
            outputs: parsed,
            output_files: BTreeMap::new(),
            next_step_id: next,
        });
    }

    let next = step
        .next
        .clone()
        .or_else(|| next_step_in_workflow(workflow, &step.id));
    let next = validate_transition_target(workflow, step, next, "step transition")?;

    Ok(StepEvaluation {
        outputs: parsed,
        output_files: BTreeMap::new(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepPromptRender {
    pub prompt: String,
    pub context: String,
}

fn resolve_json_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        let object = current.as_object()?;
        current = object.get(*segment)?;
    }
    Some(current)
}

fn value_to_rendered_text(value: &Value) -> Result<String, OrchestratorError> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    serde_json::to_string(value).map_err(|err| {
        OrchestratorError::SelectorJson(format!("failed to render placeholder value: {err}"))
    })
}

fn render_template_with_placeholders<F>(
    template: &str,
    mut resolve: F,
) -> Result<String, OrchestratorError>
where
    F: FnMut(&str) -> Result<String, OrchestratorError>,
{
    let mut rendered = String::new();
    let mut cursor = template;

    while let Some(start) = cursor.find("{{") {
        rendered.push_str(&cursor[..start]);
        let after_open = &cursor[start + 2..];
        let Some(close_offset) = after_open.find("}}") else {
            return Err(OrchestratorError::SelectorValidation(
                "unclosed placeholder in template".to_string(),
            ));
        };
        let token = after_open[..close_offset].trim();
        if token.is_empty() {
            return Err(OrchestratorError::SelectorValidation(
                "empty placeholder in template".to_string(),
            ));
        }
        rendered.push_str(&resolve(token)?);
        cursor = &after_open[close_offset + 2..];
    }

    rendered.push_str(cursor);
    Ok(rendered)
}

pub fn render_step_prompt(
    run: &WorkflowRunRecord,
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
    attempt: u32,
    run_workspace: &Path,
    output_paths: &BTreeMap<String, PathBuf>,
    step_outputs: &BTreeMap<String, Map<String, Value>>,
) -> Result<StepPromptRender, OrchestratorError> {
    let input_value = Value::Object(run.inputs.clone());
    let mut state_map = Map::from_iter([
        (
            "run_state".to_string(),
            Value::String(run.state.to_string()),
        ),
        (
            "total_iterations".to_string(),
            Value::from(run.total_iterations),
        ),
        ("started_at".to_string(), Value::from(run.started_at)),
        ("updated_at".to_string(), Value::from(run.updated_at)),
    ]);
    if let Some(step_id) = run.current_step_id.clone() {
        state_map.insert("current_step_id".to_string(), Value::String(step_id));
    }
    if let Some(current_attempt) = run.current_attempt {
        state_map.insert("current_attempt".to_string(), Value::from(current_attempt));
    }
    for (step_id, outputs) in step_outputs {
        for (key, value) in outputs {
            state_map.insert(format!("{step_id}_{key}"), value.clone());
        }
    }
    let state_value = Value::Object(state_map.clone());

    let output_schema_json = serde_json::to_string(
        &step
            .outputs
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>(),
    )
    .map_err(|err| OrchestratorError::StepPromptRender {
        step_id: step.id.clone(),
        reason: format!("failed to render output schema json: {err}"),
    })?;
    let output_paths_json = serde_json::to_string_pretty(
        &output_paths
            .iter()
            .map(|(k, v)| (k.clone(), v.display().to_string()))
            .collect::<BTreeMap<_, _>>(),
    )
    .map_err(|err| OrchestratorError::StepPromptRender {
        step_id: step.id.clone(),
        reason: format!("failed to render output paths json: {err}"),
    })?;

    let rendered_prompt = render_template_with_placeholders(&step.prompt, |token| {
        if let Some(path) = token.strip_prefix("inputs.") {
            let path_segments = path
                .split('.')
                .filter(|segment| !segment.trim().is_empty())
                .collect::<Vec<_>>();
            let Some(value) = resolve_json_path(&input_value, &path_segments) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing required placeholder `{{{{{token}}}}}`"),
                });
            };
            return value_to_rendered_text(value);
        }

        if let Some(path) = token.strip_prefix("steps.") {
            let mut segments = path.split('.').collect::<Vec<_>>();
            if segments.len() < 3 || segments[1] != "outputs" {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("unsupported placeholder `{{{{{token}}}}}`"),
                });
            }
            let source_step_id = segments.remove(0).to_string();
            let _ = segments.remove(0);
            let Some(outputs) = step_outputs.get(&source_step_id) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing outputs for placeholder `{{{{{token}}}}}`"),
                });
            };
            let output_value = Value::Object(outputs.clone());
            let Some(value) = resolve_json_path(&output_value, &segments) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing output key for placeholder `{{{{{token}}}}}`"),
                });
            };
            return value_to_rendered_text(value);
        }

        if let Some(path) = token.strip_prefix("state.") {
            let segments = path
                .split('.')
                .filter(|segment| !segment.trim().is_empty())
                .collect::<Vec<_>>();
            let Some(value) = resolve_json_path(&state_value, &segments) else {
                return Ok(String::new());
            };
            return value_to_rendered_text(value);
        }

        if token == "workflow.run_id" {
            return Ok(run.run_id.clone());
        }
        if token == "workflow.step_id" {
            return Ok(step.id.clone());
        }
        if token == "workflow.attempt" {
            return Ok(attempt.to_string());
        }
        if token == "workflow.run_workspace" {
            return Ok(run_workspace.display().to_string());
        }
        if token == "workflow.output_schema_json" {
            return Ok(output_schema_json.clone());
        }
        if token == "workflow.output_paths_json" {
            return Ok(output_paths_json.clone());
        }
        if let Some(path_key) = token.strip_prefix("workflow.output_paths.") {
            let Some(path) = output_paths.get(path_key) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing output path for placeholder `{{{{{token}}}}}`"),
                });
            };
            return Ok(path.display().to_string());
        }

        let input_field = match token {
            "workflow.channel" => Some("channel"),
            "workflow.channel_profile_id" => Some("channel_profile_id"),
            "workflow.conversation_id" => Some("conversation_id"),
            "workflow.sender_id" => Some("sender_id"),
            "workflow.selector_id" => Some("selector_id"),
            _ => None,
        };
        if let Some(field) = input_field {
            let Some(value) = run.inputs.get(field) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing required placeholder `{{{{{token}}}}}`"),
                });
            };
            return value_to_rendered_text(value);
        }

        Err(OrchestratorError::StepPromptRender {
            step_id: step.id.clone(),
            reason: format!("unsupported placeholder `{{{{{token}}}}}`"),
        })
    })?;

    let context = serde_json::to_string_pretty(&Value::Object(Map::from_iter([
        ("runId".to_string(), Value::String(run.run_id.clone())),
        ("workflowId".to_string(), Value::String(workflow.id.clone())),
        ("stepId".to_string(), Value::String(step.id.clone())),
        ("attempt".to_string(), Value::from(attempt)),
        (
            "runWorkspace".to_string(),
            Value::String(run_workspace.display().to_string()),
        ),
        ("inputs".to_string(), Value::Object(run.inputs.clone())),
        ("state".to_string(), Value::Object(state_map)),
        (
            "availableStepOutputs".to_string(),
            Value::Object(Map::from_iter(step_outputs.iter().map(
                |(step_id, outputs)| (step_id.clone(), Value::Object(outputs.clone())),
            ))),
        ),
        (
            "outputPaths".to_string(),
            Value::Object(Map::from_iter(output_paths.iter().map(|(k, path)| {
                (k.clone(), Value::String(path.display().to_string()))
            }))),
        ),
    ])))
    .map_err(|err| OrchestratorError::StepPromptRender {
        step_id: step.id.clone(),
        reason: format!("failed to render context artifact: {err}"),
    })?;

    Ok(StepPromptRender {
        prompt: rendered_prompt,
        context,
    })
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

pub fn verify_orchestrator_workspace_access(
    settings: &Settings,
    orchestrator_id: &str,
    orchestrator: &OrchestratorConfig,
) -> Result<WorkspaceAccessContext, OrchestratorError> {
    let workspace_context = resolve_workspace_access_context(settings, orchestrator_id)?;
    let requested_paths =
        collect_orchestrator_requested_paths(&workspace_context, settings, orchestrator)?;
    enforce_workspace_access(&workspace_context, &requested_paths)?;
    Ok(workspace_context)
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
    catalog: BTreeMap<String, FunctionSchema>,
    run_store: Option<WorkflowRunStore>,
    settings: Option<Settings>,
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self::new(Vec::<String>::new())
    }
}

impl FunctionRegistry {
    fn v1_catalog() -> BTreeMap<String, FunctionSchema> {
        let mut catalog = BTreeMap::new();
        let mut register = |function_id: &str,
                            description: &str,
                            args: Vec<(&str, FunctionArgType, bool, &str)>,
                            read_only: bool| {
            catalog.insert(
                function_id.to_string(),
                FunctionSchema {
                    function_id: function_id.to_string(),
                    description: description.to_string(),
                    args: args
                        .into_iter()
                        .map(|(name, arg_type, required, arg_desc)| {
                            (
                                name.to_string(),
                                FunctionArgSchema {
                                    arg_type,
                                    required,
                                    description: arg_desc.to_string(),
                                },
                            )
                        })
                        .collect(),
                    read_only,
                },
            );
        };

        register("daemon.start", "Start runtime workers", Vec::new(), false);
        register("daemon.stop", "Stop runtime workers", Vec::new(), false);
        register(
            "daemon.restart",
            "Restart runtime workers",
            Vec::new(),
            false,
        );
        register(
            "daemon.status",
            "Read runtime status and worker health",
            Vec::new(),
            true,
        );
        register("daemon.logs", "Read recent runtime logs", Vec::new(), true);
        register(
            "daemon.setup",
            "Create default config and state root",
            Vec::new(),
            false,
        );
        register(
            "daemon.send",
            "Send message to channel profile",
            vec![
                (
                    "channelProfileId",
                    FunctionArgType::String,
                    true,
                    "Target channel profile id",
                ),
                ("message", FunctionArgType::String, true, "Message content"),
            ],
            false,
        );
        register(
            "channels.reset",
            "Reset channel state directories",
            Vec::new(),
            false,
        );
        register(
            "channels.slack_sync",
            "Run one Slack sync pass",
            Vec::new(),
            false,
        );
        register(
            "provider.show",
            "Show current provider/model preferences",
            Vec::new(),
            true,
        );
        register(
            "provider.set",
            "Set provider preference and optional model",
            vec![
                (
                    "provider",
                    FunctionArgType::String,
                    true,
                    "Provider id: anthropic or openai",
                ),
                (
                    "model",
                    FunctionArgType::String,
                    false,
                    "Optional model identifier",
                ),
            ],
            false,
        );
        register(
            "model.show",
            "Show current model preference",
            Vec::new(),
            true,
        );
        register(
            "model.set",
            "Set model preference",
            vec![("model", FunctionArgType::String, true, "Model identifier")],
            false,
        );
        register(
            "agent.list",
            "List orchestrator agent ids",
            vec![(
                "orchestratorId",
                FunctionArgType::String,
                true,
                "Target orchestrator id",
            )],
            true,
        );
        register(
            "agent.add",
            "Add orchestrator-local agent",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                ("agentId", FunctionArgType::String, true, "Agent id"),
            ],
            false,
        );
        register(
            "agent.show",
            "Show orchestrator-local agent",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                ("agentId", FunctionArgType::String, true, "Agent id"),
            ],
            true,
        );
        register(
            "agent.remove",
            "Remove orchestrator-local agent",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                ("agentId", FunctionArgType::String, true, "Agent id"),
            ],
            false,
        );
        register(
            "agent.reset",
            "Reset orchestrator-local agent defaults",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                ("agentId", FunctionArgType::String, true, "Agent id"),
            ],
            false,
        );
        register(
            "orchestrator.list",
            "List orchestrator ids",
            Vec::new(),
            true,
        );
        register(
            "orchestrator.add",
            "Add orchestrator and bootstrap config",
            vec![(
                "orchestratorId",
                FunctionArgType::String,
                true,
                "Orchestrator id",
            )],
            false,
        );
        register(
            "orchestrator.show",
            "Show one orchestrator configuration summary",
            vec![(
                "orchestratorId",
                FunctionArgType::String,
                true,
                "Target orchestrator id",
            )],
            true,
        );
        register(
            "orchestrator.remove",
            "Remove orchestrator from settings",
            vec![(
                "orchestratorId",
                FunctionArgType::String,
                true,
                "Target orchestrator id",
            )],
            false,
        );
        register(
            "orchestrator.set_private_workspace",
            "Set orchestrator private workspace path",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                (
                    "path",
                    FunctionArgType::String,
                    true,
                    "Absolute private workspace path",
                ),
            ],
            false,
        );
        register(
            "orchestrator.grant_shared_access",
            "Grant shared workspace key to orchestrator",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                (
                    "sharedKey",
                    FunctionArgType::String,
                    true,
                    "Shared workspace key",
                ),
            ],
            false,
        );
        register(
            "orchestrator.revoke_shared_access",
            "Revoke shared workspace key from orchestrator",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                (
                    "sharedKey",
                    FunctionArgType::String,
                    true,
                    "Shared workspace key",
                ),
            ],
            false,
        );
        register(
            "orchestrator.set_selector_agent",
            "Set orchestrator selector agent id",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                (
                    "agentId",
                    FunctionArgType::String,
                    true,
                    "Selector agent id",
                ),
            ],
            false,
        );
        register(
            "orchestrator.set_default_workflow",
            "Set orchestrator default workflow id",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                ("workflowId", FunctionArgType::String, true, "Workflow id"),
            ],
            false,
        );
        register(
            "orchestrator.set_selection_max_retries",
            "Set selector retry limit",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                ("count", FunctionArgType::Integer, true, "Retry count >= 1"),
            ],
            false,
        );
        register(
            "workflow.list",
            "List workflows for an orchestrator",
            vec![(
                "orchestratorId",
                FunctionArgType::String,
                true,
                "Target orchestrator id",
            )],
            true,
        );
        register(
            "workflow.show",
            "Show one workflow definition",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                (
                    "workflowId",
                    FunctionArgType::String,
                    true,
                    "Workflow id in orchestrator scope",
                ),
            ],
            true,
        );
        register(
            "workflow.add",
            "Add workflow to orchestrator config",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                ("workflowId", FunctionArgType::String, true, "Workflow id"),
            ],
            false,
        );
        register(
            "workflow.remove",
            "Remove workflow from orchestrator config",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                ("workflowId", FunctionArgType::String, true, "Workflow id"),
            ],
            false,
        );
        register(
            "workflow.run",
            "Start a workflow run",
            vec![
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Target orchestrator id",
                ),
                ("workflowId", FunctionArgType::String, true, "Workflow id"),
                (
                    "inputs",
                    FunctionArgType::Object,
                    false,
                    "Optional key/value workflow inputs",
                ),
            ],
            false,
        );
        register(
            "workflow.status",
            "Read workflow run status summary",
            vec![("runId", FunctionArgType::String, true, "Workflow run id")],
            true,
        );
        register(
            "workflow.progress",
            "Read full workflow progress payload",
            vec![("runId", FunctionArgType::String, true, "Workflow run id")],
            true,
        );
        register(
            "workflow.cancel",
            "Cancel a workflow run",
            vec![("runId", FunctionArgType::String, true, "Workflow run id")],
            false,
        );
        register(
            "channel_profile.list",
            "List configured channel profile ids",
            Vec::new(),
            true,
        );
        register(
            "channel_profile.add",
            "Add channel profile mapping",
            vec![
                (
                    "channelProfileId",
                    FunctionArgType::String,
                    true,
                    "Channel profile id",
                ),
                (
                    "channel",
                    FunctionArgType::String,
                    true,
                    "Channel backend id",
                ),
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Mapped orchestrator id",
                ),
                (
                    "slackAppUserId",
                    FunctionArgType::String,
                    false,
                    "Slack bot user id",
                ),
                (
                    "requireMentionInChannels",
                    FunctionArgType::Boolean,
                    false,
                    "Slack mention requirement in channels",
                ),
            ],
            false,
        );
        register(
            "channel_profile.show",
            "Show one channel profile mapping",
            vec![(
                "channelProfileId",
                FunctionArgType::String,
                true,
                "Channel profile id",
            )],
            true,
        );
        register(
            "channel_profile.remove",
            "Remove channel profile mapping",
            vec![(
                "channelProfileId",
                FunctionArgType::String,
                true,
                "Channel profile id",
            )],
            false,
        );
        register(
            "channel_profile.set_orchestrator",
            "Update channel profile orchestrator mapping",
            vec![
                (
                    "channelProfileId",
                    FunctionArgType::String,
                    true,
                    "Channel profile id",
                ),
                (
                    "orchestratorId",
                    FunctionArgType::String,
                    true,
                    "Mapped orchestrator id",
                ),
            ],
            false,
        );
        register("update.check", "Check for updates", Vec::new(), true);
        register(
            "update.apply",
            "Apply update (unsupported in this build)",
            Vec::new(),
            false,
        );
        register(
            "daemon.attach",
            "Attach to supervisor or return workflow summary",
            Vec::new(),
            true,
        );

        catalog
    }

    fn invoke_cli(&self, args: Vec<String>) -> Result<Value, OrchestratorError> {
        let command = args.join(" ");
        let output = cli::run(args).map_err(OrchestratorError::SelectorValidation)?;
        Ok(Value::Object(Map::from_iter([
            ("command".to_string(), Value::String(command)),
            ("output".to_string(), Value::String(output)),
        ])))
    }

    fn normalize_allowlist<I>(
        function_ids: I,
        catalog: &BTreeMap<String, FunctionSchema>,
    ) -> BTreeSet<String>
    where
        I: IntoIterator<Item = String>,
    {
        function_ids
            .into_iter()
            .filter(|id| catalog.contains_key(id))
            .collect()
    }

    pub fn new<I>(function_ids: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let catalog = Self::v1_catalog();
        Self {
            allowed: Self::normalize_allowlist(function_ids, &catalog),
            catalog,
            run_store: None,
            settings: None,
        }
    }

    pub fn with_run_store<I>(function_ids: I, run_store: WorkflowRunStore) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let catalog = Self::v1_catalog();
        Self {
            allowed: Self::normalize_allowlist(function_ids, &catalog),
            catalog,
            run_store: Some(run_store),
            settings: None,
        }
    }

    pub fn with_context<I>(
        function_ids: I,
        run_store: Option<WorkflowRunStore>,
        settings: Option<Settings>,
    ) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let catalog = Self::v1_catalog();
        Self {
            allowed: Self::normalize_allowlist(function_ids, &catalog),
            catalog,
            run_store,
            settings,
        }
    }

    pub fn v1_defaults(run_store: WorkflowRunStore, settings: &Settings) -> Self {
        let catalog = Self::v1_catalog();
        let allowed = catalog.keys().cloned().collect();
        Self {
            allowed,
            catalog,
            run_store: Some(run_store),
            settings: Some(settings.clone()),
        }
    }

    pub fn contains(&self, function_id: &str) -> bool {
        self.allowed.contains(function_id)
    }

    pub fn available_function_ids(&self) -> Vec<String> {
        self.allowed.iter().cloned().collect()
    }

    pub fn available_function_schemas(&self) -> Vec<FunctionSchema> {
        self.allowed
            .iter()
            .filter_map(|id| self.catalog.get(id))
            .cloned()
            .collect()
    }

    fn validate_args(
        &self,
        call: &FunctionCall,
        schema: &FunctionSchema,
    ) -> Result<(), OrchestratorError> {
        for key in call.args.keys() {
            if !schema.args.contains_key(key) {
                return Err(OrchestratorError::UnknownFunctionArg {
                    function_id: call.function_id.clone(),
                    arg: key.clone(),
                });
            }
        }
        for (arg, arg_schema) in &schema.args {
            match call.args.get(arg) {
                Some(value) => {
                    if !arg_schema.arg_type.matches(value) {
                        return Err(OrchestratorError::InvalidFunctionArgType {
                            function_id: call.function_id.clone(),
                            arg: arg.clone(),
                            expected: arg_schema.arg_type.to_string(),
                        });
                    }
                }
                None if arg_schema.required => {
                    return Err(OrchestratorError::MissingFunctionArg { arg: arg.clone() })
                }
                None => {}
            }
        }
        Ok(())
    }

    pub fn invoke(&self, call: &FunctionCall) -> Result<Value, OrchestratorError> {
        if !self.contains(&call.function_id) {
            return Err(OrchestratorError::UnknownFunction {
                function_id: call.function_id.clone(),
            });
        }
        let schema = self.catalog.get(&call.function_id).ok_or_else(|| {
            OrchestratorError::UnknownFunction {
                function_id: call.function_id.clone(),
            }
        })?;
        self.validate_args(call, schema)?;

        match call.function_id.as_str() {
            "daemon.start" => self.invoke_cli(vec!["start".to_string()]),
            "daemon.stop" => self.invoke_cli(vec!["stop".to_string()]),
            "daemon.restart" => self.invoke_cli(vec!["restart".to_string()]),
            "daemon.status" => self.invoke_cli(vec!["status".to_string()]),
            "daemon.logs" => self.invoke_cli(vec!["logs".to_string()]),
            "daemon.setup" => self.invoke_cli(vec!["setup".to_string()]),
            "daemon.send" => {
                let profile_id = parse_required_string_arg(&call.args, "channelProfileId")?;
                let message = parse_required_string_arg(&call.args, "message")?;
                self.invoke_cli(vec!["send".to_string(), profile_id, message])
            }
            "channels.reset" => self.invoke_cli(vec!["channels".to_string(), "reset".to_string()]),
            "channels.slack_sync" => self.invoke_cli(vec![
                "channels".to_string(),
                "slack".to_string(),
                "sync".to_string(),
            ]),
            "provider.show" => self.invoke_cli(vec!["provider".to_string()]),
            "provider.set" => {
                let provider = parse_required_string_arg(&call.args, "provider")?;
                let mut args = vec!["provider".to_string(), provider];
                if let Some(model) = parse_optional_string_arg(&call.args, "model")? {
                    args.push("--model".to_string());
                    args.push(model);
                }
                self.invoke_cli(args)
            }
            "model.show" => self.invoke_cli(vec!["model".to_string()]),
            "model.set" => {
                let model = parse_required_string_arg(&call.args, "model")?;
                self.invoke_cli(vec!["model".to_string(), model])
            }
            "agent.list" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                self.invoke_cli(vec![
                    "agent".to_string(),
                    "list".to_string(),
                    orchestrator_id,
                ])
            }
            "agent.add" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let agent_id = parse_required_string_arg(&call.args, "agentId")?;
                self.invoke_cli(vec![
                    "agent".to_string(),
                    "add".to_string(),
                    orchestrator_id,
                    agent_id,
                ])
            }
            "agent.show" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let agent_id = parse_required_string_arg(&call.args, "agentId")?;
                self.invoke_cli(vec![
                    "agent".to_string(),
                    "show".to_string(),
                    orchestrator_id,
                    agent_id,
                ])
            }
            "agent.remove" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let agent_id = parse_required_string_arg(&call.args, "agentId")?;
                self.invoke_cli(vec![
                    "agent".to_string(),
                    "remove".to_string(),
                    orchestrator_id,
                    agent_id,
                ])
            }
            "agent.reset" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let agent_id = parse_required_string_arg(&call.args, "agentId")?;
                self.invoke_cli(vec![
                    "agent".to_string(),
                    "reset".to_string(),
                    orchestrator_id,
                    agent_id,
                ])
            }
            "orchestrator.add" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                self.invoke_cli(vec![
                    "orchestrator".to_string(),
                    "add".to_string(),
                    orchestrator_id,
                ])
            }
            "workflow.list" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let settings = self.settings.as_ref().ok_or_else(|| {
                    OrchestratorError::SelectorValidation(
                        "workflow.list requires settings context".to_string(),
                    )
                })?;
                let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
                Ok(Value::Object(Map::from_iter([
                    ("orchestratorId".to_string(), Value::String(orchestrator_id)),
                    (
                        "workflows".to_string(),
                        Value::Array(
                            orchestrator
                                .workflows
                                .iter()
                                .map(|w| Value::String(w.id.clone()))
                                .collect(),
                        ),
                    ),
                ])))
            }
            "workflow.show" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let workflow_id = parse_required_string_arg(&call.args, "workflowId")?;
                let settings = self.settings.as_ref().ok_or_else(|| {
                    OrchestratorError::SelectorValidation(
                        "workflow.show requires settings context".to_string(),
                    )
                })?;
                let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
                let workflow = orchestrator
                    .workflows
                    .iter()
                    .find(|w| w.id == workflow_id)
                    .ok_or_else(|| {
                        OrchestratorError::SelectorValidation(format!(
                            "workflow `{workflow_id}` not found in orchestrator `{orchestrator_id}`"
                        ))
                    })?;
                Ok(Value::Object(Map::from_iter([
                    ("orchestratorId".to_string(), Value::String(orchestrator_id)),
                    ("workflowId".to_string(), Value::String(workflow_id)),
                    (
                        "workflow".to_string(),
                        serde_json::to_value(workflow)
                            .map_err(|e| OrchestratorError::SelectorJson(e.to_string()))?,
                    ),
                ])))
            }
            "workflow.add" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let workflow_id = parse_required_string_arg(&call.args, "workflowId")?;
                self.invoke_cli(vec![
                    "workflow".to_string(),
                    "add".to_string(),
                    orchestrator_id,
                    workflow_id,
                ])
            }
            "workflow.remove" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let workflow_id = parse_required_string_arg(&call.args, "workflowId")?;
                self.invoke_cli(vec![
                    "workflow".to_string(),
                    "remove".to_string(),
                    orchestrator_id,
                    workflow_id,
                ])
            }
            "workflow.run" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let workflow_id = parse_required_string_arg(&call.args, "workflowId")?;
                let mut args = vec![
                    "workflow".to_string(),
                    "run".to_string(),
                    orchestrator_id,
                    workflow_id,
                ];
                if let Some(inputs) = parse_optional_object_arg(&call.args, "inputs") {
                    for (key, value) in inputs {
                        let encoded = value
                            .as_str()
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| value.to_string());
                        args.push("--input".to_string());
                        args.push(format!("{key}={encoded}"));
                    }
                }
                self.invoke_cli(args)
            }
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
            "workflow.progress" => {
                let run_id = parse_run_id_arg(&call.args)?;
                let run_store = self.run_store.as_ref().ok_or_else(|| {
                    OrchestratorError::SelectorValidation(
                        "workflow.progress requires workflow run store".to_string(),
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
            "orchestrator.list" => {
                let settings = self.settings.as_ref().ok_or_else(|| {
                    OrchestratorError::SelectorValidation(
                        "orchestrator.list requires settings context".to_string(),
                    )
                })?;
                Ok(Value::Object(Map::from_iter([(
                    "orchestrators".to_string(),
                    Value::Array(
                        settings
                            .orchestrators
                            .keys()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                )])))
            }
            "orchestrator.show" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let settings = self.settings.as_ref().ok_or_else(|| {
                    OrchestratorError::SelectorValidation(
                        "orchestrator.show requires settings context".to_string(),
                    )
                })?;
                let entry = settings
                    .orchestrators
                    .get(&orchestrator_id)
                    .ok_or_else(|| {
                        OrchestratorError::SelectorValidation(format!(
                            "unknown orchestrator `{orchestrator_id}`"
                        ))
                    })?;
                let private_workspace = settings.resolve_private_workspace(&orchestrator_id)?;
                Ok(Value::Object(Map::from_iter([
                    ("orchestratorId".to_string(), Value::String(orchestrator_id)),
                    (
                        "privateWorkspace".to_string(),
                        Value::String(private_workspace.display().to_string()),
                    ),
                    (
                        "sharedAccess".to_string(),
                        Value::Array(
                            entry
                                .shared_access
                                .iter()
                                .cloned()
                                .map(Value::String)
                                .collect(),
                        ),
                    ),
                ])))
            }
            "orchestrator.remove" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                self.invoke_cli(vec![
                    "orchestrator".to_string(),
                    "remove".to_string(),
                    orchestrator_id,
                ])
            }
            "orchestrator.set_private_workspace" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let path = parse_required_string_arg(&call.args, "path")?;
                self.invoke_cli(vec![
                    "orchestrator".to_string(),
                    "set-private-workspace".to_string(),
                    orchestrator_id,
                    path,
                ])
            }
            "orchestrator.grant_shared_access" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let shared_key = parse_required_string_arg(&call.args, "sharedKey")?;
                self.invoke_cli(vec![
                    "orchestrator".to_string(),
                    "grant-shared-access".to_string(),
                    orchestrator_id,
                    shared_key,
                ])
            }
            "orchestrator.revoke_shared_access" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let shared_key = parse_required_string_arg(&call.args, "sharedKey")?;
                self.invoke_cli(vec![
                    "orchestrator".to_string(),
                    "revoke-shared-access".to_string(),
                    orchestrator_id,
                    shared_key,
                ])
            }
            "orchestrator.set_selector_agent" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let agent_id = parse_required_string_arg(&call.args, "agentId")?;
                self.invoke_cli(vec![
                    "orchestrator".to_string(),
                    "set-selector-agent".to_string(),
                    orchestrator_id,
                    agent_id,
                ])
            }
            "orchestrator.set_default_workflow" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let workflow_id = parse_required_string_arg(&call.args, "workflowId")?;
                self.invoke_cli(vec![
                    "orchestrator".to_string(),
                    "set-default-workflow".to_string(),
                    orchestrator_id,
                    workflow_id,
                ])
            }
            "orchestrator.set_selection_max_retries" => {
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let count = parse_required_u32_arg(&call.args, "count")?;
                self.invoke_cli(vec![
                    "orchestrator".to_string(),
                    "set-selection-max-retries".to_string(),
                    orchestrator_id,
                    count.to_string(),
                ])
            }
            "channel_profile.list" => {
                let settings = self.settings.as_ref().ok_or_else(|| {
                    OrchestratorError::SelectorValidation(
                        "channel_profile.list requires settings context".to_string(),
                    )
                })?;
                Ok(Value::Object(Map::from_iter([(
                    "channelProfiles".to_string(),
                    Value::Array(
                        settings
                            .channel_profiles
                            .keys()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                )])))
            }
            "channel_profile.show" => {
                let profile_id = parse_required_string_arg(&call.args, "channelProfileId")?;
                let settings = self.settings.as_ref().ok_or_else(|| {
                    OrchestratorError::SelectorValidation(
                        "channel_profile.show requires settings context".to_string(),
                    )
                })?;
                let profile = settings.channel_profiles.get(&profile_id).ok_or_else(|| {
                    OrchestratorError::SelectorValidation(format!(
                        "unknown channel profile `{profile_id}`"
                    ))
                })?;
                Ok(Value::Object(Map::from_iter([
                    ("channelProfileId".to_string(), Value::String(profile_id)),
                    (
                        "channel".to_string(),
                        Value::String(profile.channel.clone()),
                    ),
                    (
                        "orchestratorId".to_string(),
                        Value::String(profile.orchestrator_id.clone()),
                    ),
                    (
                        "slackAppUserId".to_string(),
                        profile
                            .slack_app_user_id
                            .as_ref()
                            .map(|v| Value::String(v.clone()))
                            .unwrap_or(Value::Null),
                    ),
                    (
                        "requireMentionInChannels".to_string(),
                        profile
                            .require_mention_in_channels
                            .map(Value::Bool)
                            .unwrap_or(Value::Null),
                    ),
                ])))
            }
            "channel_profile.add" => {
                let profile_id = parse_required_string_arg(&call.args, "channelProfileId")?;
                let channel = parse_required_string_arg(&call.args, "channel")?;
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                let mut args = vec![
                    "channel-profile".to_string(),
                    "add".to_string(),
                    profile_id,
                    channel,
                    orchestrator_id,
                ];
                if let Some(user_id) = parse_optional_string_arg(&call.args, "slackAppUserId")? {
                    args.push("--slack-app-user-id".to_string());
                    args.push(user_id);
                }
                if let Some(require_mention) =
                    parse_optional_bool_arg(&call.args, "requireMentionInChannels")?
                {
                    args.push("--require-mention-in-channels".to_string());
                    args.push(require_mention.to_string());
                }
                self.invoke_cli(args)
            }
            "channel_profile.remove" => {
                let profile_id = parse_required_string_arg(&call.args, "channelProfileId")?;
                self.invoke_cli(vec![
                    "channel-profile".to_string(),
                    "remove".to_string(),
                    profile_id,
                ])
            }
            "channel_profile.set_orchestrator" => {
                let profile_id = parse_required_string_arg(&call.args, "channelProfileId")?;
                let orchestrator_id = parse_required_string_arg(&call.args, "orchestratorId")?;
                self.invoke_cli(vec![
                    "channel-profile".to_string(),
                    "set-orchestrator".to_string(),
                    profile_id,
                    orchestrator_id,
                ])
            }
            "update.check" => self.invoke_cli(vec!["update".to_string(), "check".to_string()]),
            "update.apply" => self.invoke_cli(vec!["update".to_string(), "apply".to_string()]),
            "daemon.attach" => self.invoke_cli(vec!["attach".to_string()]),
            _ => Err(OrchestratorError::SelectorValidation(format!(
                "function `{}` is allowed but has no implementation",
                call.function_id
            ))),
        }
    }
}

fn parse_run_id_arg(args: &Map<String, Value>) -> Result<String, OrchestratorError> {
    parse_required_string_arg(args, "runId")
}

fn parse_required_string_arg(
    args: &Map<String, Value>,
    arg: &str,
) -> Result<String, OrchestratorError> {
    args.get(arg)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| OrchestratorError::MissingFunctionArg {
            arg: arg.to_string(),
        })
}

fn parse_optional_string_arg(
    args: &Map<String, Value>,
    arg: &str,
) -> Result<Option<String>, OrchestratorError> {
    match args.get(arg) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        Some(Value::String(_)) => Err(OrchestratorError::MissingFunctionArg {
            arg: arg.to_string(),
        }),
        Some(_) => Err(OrchestratorError::SelectorValidation(format!(
            "argument `{arg}` must be a non-empty string"
        ))),
        None => Ok(None),
    }
}

fn parse_optional_bool_arg(
    args: &Map<String, Value>,
    arg: &str,
) -> Result<Option<bool>, OrchestratorError> {
    match args.get(arg) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(OrchestratorError::SelectorValidation(format!(
            "argument `{arg}` must be boolean"
        ))),
        None => Ok(None),
    }
}

fn parse_required_u32_arg(args: &Map<String, Value>, arg: &str) -> Result<u32, OrchestratorError> {
    let value = args.get(arg).and_then(Value::as_u64).ok_or_else(|| {
        OrchestratorError::MissingFunctionArg {
            arg: arg.to_string(),
        }
    })?;
    u32::try_from(value).map_err(|_| {
        OrchestratorError::SelectorValidation(format!("argument `{arg}` is out of range for u32"))
    })
}

fn parse_optional_object_arg<'a>(
    args: &'a Map<String, Value>,
    arg: &str,
) -> Option<&'a Map<String, Value>> {
    args.get(arg).and_then(Value::as_object)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResolutionInput {
    pub explicit_run_id: Option<String>,
    pub inbound_workflow_run_id: Option<String>,
    pub channel_profile_id: Option<String>,
    pub conversation_id: Option<String>,
}

fn selector_start_inputs(
    request: &SelectorRequest,
    source_message_id: Option<&str>,
) -> Map<String, Value> {
    let mut inputs = Map::new();
    inputs.insert(
        "user_message".to_string(),
        Value::String(request.user_message.clone()),
    );
    inputs.insert(
        "channel_profile_id".to_string(),
        Value::String(request.channel_profile_id.clone()),
    );
    inputs.insert(
        "selector_id".to_string(),
        Value::String(request.selector_id.clone()),
    );
    inputs.insert(
        "message_id".to_string(),
        Value::String(request.message_id.clone()),
    );
    if let Some(conversation_id) = request.conversation_id.as_ref() {
        inputs.insert(
            "conversation_id".to_string(),
            Value::String(conversation_id.clone()),
        );
    }
    if let Some(source_message_id) = source_message_id {
        inputs.insert(
            "source_message_id".to_string(),
            Value::String(source_message_id.to_string()),
        );
    }
    inputs
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
    pub workspace_access_context: Option<WorkspaceAccessContext>,
    pub runner_binaries: Option<RunnerBinaries>,
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
            ctx.run_store.create_run_with_metadata(
                run_id.clone(),
                workflow_id.clone(),
                SelectorStartedRunMetadata {
                    source_message_id: ctx.source_message_id.map(|v| v.to_string()),
                    selector_id: Some(request.selector_id.clone()),
                    selected_workflow: Some(workflow_id.clone()),
                    status_conversation_id: request.conversation_id.clone(),
                },
                selector_start_inputs(request, ctx.source_message_id),
                ctx.now,
            )?;
            let mut engine = WorkflowEngine::new(ctx.run_store.clone(), ctx.orchestrator.clone());
            if let Some(context) = ctx.workspace_access_context.clone() {
                engine = engine.with_workspace_access_context(context);
            }
            if let Some(binaries) = ctx.runner_binaries.clone() {
                engine = engine.with_runner_binaries(binaries);
            }
            engine.start(&run_id, ctx.now)?;

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
    next_selector_attempt: F,
) -> Result<RoutedSelectorAction, OrchestratorError>
where
    F: FnMut(u32, &SelectorRequest, &OrchestratorConfig) -> Option<String>,
{
    process_queued_message_with_runner_binaries(
        state_root,
        settings,
        inbound,
        now,
        active_conversation_runs,
        functions,
        None,
        next_selector_attempt,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn process_queued_message_with_runner_binaries<F>(
    state_root: &Path,
    settings: &Settings,
    inbound: &IncomingMessage,
    now: i64,
    active_conversation_runs: &BTreeMap<(String, String), String>,
    functions: &FunctionRegistry,
    runner_binaries: Option<RunnerBinaries>,
    mut next_selector_attempt: F,
) -> Result<RoutedSelectorAction, OrchestratorError>
where
    F: FnMut(u32, &SelectorRequest, &OrchestratorConfig) -> Option<String>,
{
    let runner_binaries = runner_binaries.unwrap_or_else(resolve_runner_binaries);
    let run_store = WorkflowRunStore::new(state_root);
    let inbound_message = inbound.message.trim().to_ascii_lowercase();
    if let Some(run_id) = inbound
        .workflow_run_id
        .as_ref()
        .filter(|v| !v.trim().is_empty())
    {
        if matches!(
            inbound_message.as_str(),
            "status" | "progress" | "/status" | "/progress"
        ) {
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
                available_function_schemas: functions.available_function_schemas(),
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
                        selector_timeout_seconds: 30,
                        agents: BTreeMap::new(),
                        workflows: Vec::new(),
                        workflow_orchestration: None,
                    },
                    workspace_access_context: None,
                    runner_binaries: Some(runner_binaries.clone()),
                    source_message_id: Some(&inbound.message_id),
                    now,
                },
            );
        }

        let orchestrator_id = resolve_orchestrator_id(settings, inbound)?;
        let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
        let workspace_context = match verify_orchestrator_workspace_access(
            settings,
            &orchestrator_id,
            &orchestrator,
        ) {
            Ok(context) => context,
            Err(err) => {
                append_security_log(
                        state_root,
                        &format!(
                            "workspace access denied for orchestrator `{orchestrator_id}` message `{}`: {err}",
                            inbound.message_id
                        ),
                    );
                return Err(err);
            }
        };

        let engine = WorkflowEngine::new(run_store.clone(), orchestrator.clone())
            .with_runner_binaries(runner_binaries.clone())
            .with_workspace_access_context(workspace_context);
        let resumed = match engine
            .resume(run_id, now)
            .map_err(|e| missing_run_for_io(run_id, &e).unwrap_or(e))
        {
            Ok(run) => run,
            Err(OrchestratorError::UnknownRunId { .. }) => {
                return Ok(RoutedSelectorAction::WorkflowStatus {
                    run_id: Some(run_id.to_string()),
                    progress: None,
                    message: format!("workflow run `{run_id}` was not found"),
                });
            }
            Err(err) => return Err(err),
        };
        let progress = match run_store
            .load_progress(run_id)
            .map_err(|e| missing_run_for_io(run_id, &e).unwrap_or(e))
        {
            Ok(progress) => progress,
            Err(OrchestratorError::UnknownRunId { .. }) => {
                return Ok(RoutedSelectorAction::WorkflowStatus {
                    run_id: Some(run_id.to_string()),
                    progress: None,
                    message: format!("workflow run `{run_id}` was not found"),
                });
            }
            Err(err) => return Err(err),
        };
        return Ok(RoutedSelectorAction::WorkflowStatus {
            run_id: Some(resumed.run_id),
            progress: Some(progress),
            message: "workflow progress loaded".to_string(),
        });
    }

    let orchestrator_id = resolve_orchestrator_id(settings, inbound)?;
    let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
    let workspace_context =
        match verify_orchestrator_workspace_access(settings, &orchestrator_id, &orchestrator) {
            Ok(context) => context,
            Err(err) => {
                append_security_log(
                    state_root,
                    &format!(
                "workspace access denied for orchestrator `{orchestrator_id}` message `{}`: {err}",
                inbound.message_id
            ),
                );
                return Err(err);
            }
        };

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
        available_function_schemas: functions.available_function_schemas(),
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
            workspace_access_context: Some(workspace_context),
            runner_binaries: Some(runner_binaries),
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
workspaces_path: /tmp/workspace
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
            available_function_schemas: Vec::new(),
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
    fn workflow_run_record_inputs_round_trip_and_backward_compat() {
        let run = WorkflowRunRecord {
            run_id: "run-inputs".to_string(),
            workflow_id: "wf".to_string(),
            state: RunState::Running,
            inputs: Map::from_iter([("ticket".to_string(), Value::String("123".to_string()))]),
            current_step_id: Some("step-1".to_string()),
            current_attempt: Some(1),
            started_at: 10,
            updated_at: 11,
            total_iterations: 1,
            source_message_id: None,
            selector_id: None,
            selected_workflow: None,
            status_conversation_id: None,
            terminal_reason: None,
        };
        let encoded = serde_json::to_string(&run).expect("encode");
        let decoded: WorkflowRunRecord = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(
            decoded.inputs.get("ticket"),
            Some(&Value::String("123".to_string()))
        );

        let legacy = r#"{
          "runId":"legacy-run",
          "workflowId":"wf",
          "state":"queued",
          "startedAt":1,
          "updatedAt":1,
          "totalIterations":0
        }"#;
        let legacy_decoded: WorkflowRunRecord = serde_json::from_str(legacy).expect("legacy");
        assert!(legacy_decoded.inputs.is_empty());
    }

    #[test]
    fn workspace_access_context_and_enforcement_allow_private_and_granted_shared_only() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
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
                PathBuf::from("/tmp/workspace/alpha/agents/worker"),
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

    #[test]
    fn function_registry_exposes_machine_readable_schemas_for_v1_scope() {
        let expected_ids = vec![
            "daemon.start",
            "daemon.stop",
            "daemon.restart",
            "daemon.status",
            "daemon.logs",
            "daemon.setup",
            "daemon.send",
            "channels.reset",
            "channels.slack_sync",
            "provider.show",
            "provider.set",
            "model.show",
            "model.set",
            "agent.list",
            "agent.add",
            "agent.show",
            "agent.remove",
            "agent.reset",
            "orchestrator.list",
            "orchestrator.add",
            "orchestrator.show",
            "orchestrator.remove",
            "orchestrator.set_private_workspace",
            "orchestrator.grant_shared_access",
            "orchestrator.revoke_shared_access",
            "orchestrator.set_selector_agent",
            "orchestrator.set_default_workflow",
            "orchestrator.set_selection_max_retries",
            "workflow.list",
            "workflow.show",
            "workflow.add",
            "workflow.remove",
            "workflow.run",
            "workflow.status",
            "workflow.progress",
            "workflow.cancel",
            "channel_profile.list",
            "channel_profile.add",
            "channel_profile.show",
            "channel_profile.remove",
            "channel_profile.set_orchestrator",
            "update.check",
            "update.apply",
            "daemon.attach",
        ];
        let registry = FunctionRegistry::new(expected_ids.iter().map(|id| id.to_string()));
        let schemas = registry.available_function_schemas();
        assert_eq!(schemas.len(), expected_ids.len());
        for expected in &expected_ids {
            assert!(
                schemas.iter().any(|f| &f.function_id == expected),
                "missing function schema for {expected}"
            );
        }
        assert!(schemas
            .iter()
            .any(|f| f.function_id == "workflow.progress" && f.read_only));
        assert!(schemas.iter().any(|f| {
            f.function_id == "workflow.cancel" && !f.read_only && f.args.contains_key("runId")
        }));
        assert!(schemas.iter().any(
            |f| f.function_id == "orchestrator.set_selection_max_retries"
                && f.args.contains_key("count")
        ));
    }

    #[test]
    fn function_registry_rejects_unknown_and_invalid_args() {
        let registry = FunctionRegistry::new(vec!["workflow.status".to_string()]);
        let unknown_arg = FunctionCall {
            function_id: "workflow.status".to_string(),
            args: Map::from_iter([("extra".to_string(), Value::String("x".to_string()))]),
        };
        let err = registry.invoke(&unknown_arg).expect_err("unknown arg");
        assert!(err.to_string().contains("unknown function argument"));

        let invalid_type = FunctionCall {
            function_id: "workflow.status".to_string(),
            args: Map::from_iter([("runId".to_string(), Value::Bool(true))]),
        };
        let err = registry.invoke(&invalid_type).expect_err("invalid type");
        assert!(err.to_string().contains("invalid argument type"));
    }

    #[test]
    fn workflow_status_and_progress_commands_are_read_only() {
        let temp = tempdir().expect("tempdir");
        let store = WorkflowRunStore::new(temp.path());
        let run_id = "run-readonly";
        let mut run = store.create_run(run_id, "wf", 10).expect("create run");
        store
            .transition_state(
                &mut run,
                RunState::Running,
                11,
                "running",
                false,
                "continue",
            )
            .expect("running");
        let before = store.load_run(run_id).expect("before");

        let registry = FunctionRegistry::with_run_store(
            vec![
                "workflow.status".to_string(),
                "workflow.progress".to_string(),
            ],
            store.clone(),
        );
        registry
            .invoke(&FunctionCall {
                function_id: "workflow.status".to_string(),
                args: Map::from_iter([("runId".to_string(), Value::String(run_id.to_string()))]),
            })
            .expect("status call");
        registry
            .invoke(&FunctionCall {
                function_id: "workflow.progress".to_string(),
                args: Map::from_iter([("runId".to_string(), Value::String(run_id.to_string()))]),
            })
            .expect("progress call");

        let after = store.load_run(run_id).expect("after");
        assert_eq!(before.updated_at, after.updated_at);
        assert_eq!(before.state, after.state);
        assert_eq!(before.current_step_id, after.current_step_id);
        assert_eq!(before.current_attempt, after.current_attempt);
    }
}
