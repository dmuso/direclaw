use crate::config::ConfigError;
use crate::orchestration::run_store::RunState;

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
    #[error(
        "scheduled execution completion failed for job `{job_id}` execution `{execution_id}`: {reason}"
    )]
    ScheduledExecutionCompletion {
        job_id: String,
        execution_id: String,
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
