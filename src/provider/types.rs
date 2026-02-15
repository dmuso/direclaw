use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("unknown provider `{0}`")]
    UnknownProvider(String),
    #[error("unsupported anthropic model `{0}`")]
    UnsupportedAnthropicModel(String),
    #[error("provider binary missing for {provider}: {binary}")]
    MissingBinary {
        provider: ProviderKind,
        binary: String,
        log: Box<InvocationLog>,
    },
    #[error("provider process failed for {provider} with exit code {exit_code}: {stderr}")]
    NonZeroExit {
        provider: ProviderKind,
        exit_code: i32,
        stderr: String,
        log: Box<InvocationLog>,
    },
    #[error("provider process timed out for {provider} after {timeout_ms}ms")]
    Timeout {
        provider: ProviderKind,
        timeout_ms: u64,
        log: Box<InvocationLog>,
    },
    #[error("provider output parse failure for {provider}: {reason}")]
    ParseFailure {
        provider: ProviderKind,
        reason: String,
        log: Option<Box<InvocationLog>>,
    },
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::Anthropic => write!(f, "anthropic"),
            ProviderKind::OpenAi => write!(f, "openai"),
        }
    }
}

impl TryFrom<&str> for ProviderKind {
    type Error = ProviderError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            other => Err(ProviderError::UnknownProvider(other.to_string())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromptArtifacts {
    pub prompt_file: PathBuf,
    pub context_files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub agent_id: String,
    pub provider: ProviderKind,
    pub model: String,
    pub cwd: PathBuf,
    pub message: String,
    pub prompt_artifacts: PromptArtifacts,
    pub timeout: Duration,
    pub reset_requested: bool,
    pub fresh_on_failure: bool,
    pub env_overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct InvocationSpec {
    pub binary: String,
    pub args: Vec<String>,
    pub resolved_model: String,
}

#[derive(Debug, Clone)]
pub struct InvocationLog {
    pub agent_id: String,
    pub provider: ProviderKind,
    pub model: String,
    pub command_form: String,
    pub working_directory: PathBuf,
    pub prompt_file: PathBuf,
    pub context_files: Vec<PathBuf>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderResult {
    pub message: String,
    pub log: InvocationLog,
}

pub(crate) fn io_error(path: &Path, source: std::io::Error) -> ProviderError {
    ProviderError::Io {
        path: path.display().to_string(),
        source,
    }
}
