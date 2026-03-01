use crate::orchestration::error::OrchestratorError;
pub use crate::orchestration::progress::ProgressSnapshot;
use crate::shared::logging::{append_orchestrator_log_line, orchestrator_log_path};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMemoryContext {
    #[serde(default)]
    pub bulletin: String,
    #[serde(default)]
    pub citations: Vec<String>,
}

impl RunMemoryContext {
    pub fn from_selector_request(bulletin: Option<&str>, citations: &[String]) -> Self {
        Self {
            bulletin: bulletin.unwrap_or_default().to_string(),
            citations: citations.to_vec(),
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
    pub channel_profile_id: Option<String>,
    #[serde(default)]
    pub inputs: Map<String, Value>,
    #[serde(default)]
    pub memory_context: RunMemoryContext,
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
    pub final_output_priority: Vec<String>,
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
    pub channel_profile_id: Option<String>,
    pub status_conversation_id: Option<String>,
    pub memory_context: RunMemoryContext,
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
            channel_profile_id: metadata.channel_profile_id,
            inputs,
            memory_context: metadata.memory_context,
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
        let raw = fs::read_to_string(&path).map_err(|err| io_error(&path, err))?;
        serde_json::from_str(&raw).map_err(|e| json_error(&path, e))
    }

    pub fn persist_run(&self, run: &WorkflowRunRecord) -> Result<(), OrchestratorError> {
        let path = self.run_metadata_path(&run.run_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        let body = serde_json::to_vec_pretty(run).map_err(|e| json_error(&path, e))?;
        fs::write(&path, &body).map_err(|e| io_error(&path, e))
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

    pub fn append_engine_log(
        &self,
        run_id: &str,
        now: i64,
        message: impl AsRef<str>,
    ) -> Result<(), OrchestratorError> {
        let line = format!("ts={now} run_id={run_id} {}", message.as_ref());
        append_orchestrator_log_line(&self.state_root, &line)
            .map_err(|source| io_error(orchestrator_log_path(&self.state_root).as_path(), source))
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

    pub fn latest_run_for_source_message_id(
        &self,
        source_message_id: &str,
    ) -> Result<Option<WorkflowRunRecord>, OrchestratorError> {
        let runs_root = self.state_root.join("workflows/runs");
        let entries = match fs::read_dir(&runs_root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io_error(&runs_root, source)),
        };

        let mut latest: Option<WorkflowRunRecord> = None;
        for entry in entries {
            let entry = entry.map_err(|source| io_error(&runs_root, source))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let raw = fs::read_to_string(&path).map_err(|source| io_error(&path, source))?;
            let run: WorkflowRunRecord =
                serde_json::from_str(&raw).map_err(|source| json_error(&path, source))?;
            if run.source_message_id.as_deref() != Some(source_message_id) {
                continue;
            }
            let should_replace = latest
                .as_ref()
                .map(|current| {
                    run.updated_at > current.updated_at
                        || (run.updated_at == current.updated_at
                            && run.started_at > current.started_at)
                })
                .unwrap_or(true);
            if should_replace {
                latest = Some(run);
            }
        }
        Ok(latest)
    }

    pub fn latest_run_for_conversation(
        &self,
        channel_profile_id: &str,
        conversation_id: &str,
        include_terminal: bool,
    ) -> Result<Option<WorkflowRunRecord>, OrchestratorError> {
        let runs_root = self.state_root.join("workflows/runs");
        let entries = match fs::read_dir(&runs_root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io_error(&runs_root, source)),
        };

        let mut latest: Option<WorkflowRunRecord> = None;
        for entry in entries {
            let entry = entry.map_err(|source| io_error(&runs_root, source))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let raw = fs::read_to_string(&path).map_err(|source| io_error(&path, source))?;
            let run: WorkflowRunRecord =
                serde_json::from_str(&raw).map_err(|source| json_error(&path, source))?;
            if run.channel_profile_id.as_deref() != Some(channel_profile_id) {
                continue;
            }
            if run.status_conversation_id.as_deref() != Some(conversation_id) {
                continue;
            }
            if !include_terminal && run.state.clone().is_terminal() {
                continue;
            }
            let should_replace = latest
                .as_ref()
                .map(|current| {
                    run.updated_at > current.updated_at
                        || (run.updated_at == current.updated_at
                            && run.started_at > current.started_at)
                })
                .unwrap_or(true);
            if should_replace {
                latest = Some(run);
            }
        }

        Ok(latest)
    }
}

fn sorted_input_keys(inputs: &Map<String, Value>) -> Vec<String> {
    let mut keys = inputs.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
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
