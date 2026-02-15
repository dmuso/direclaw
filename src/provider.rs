use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub mod invocation;
pub mod model_map;
pub mod output_parse;
pub mod runner;

pub use invocation::build_invocation;
pub use model_map::resolve_anthropic_model;
pub use output_parse::parse_openai_jsonl;
pub use runner::{run_provider, RunnerBinaries};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetResolution {
    pub reset_requested: bool,
    pub consumed_agent: bool,
}

pub fn consume_reset_flag(agent_flag: &Path) -> Result<ResetResolution, ProviderError> {
    let mut consumed_agent = false;

    if agent_flag.exists() {
        fs::remove_file(agent_flag).map_err(|err| io_error(agent_flag, err))?;
        consumed_agent = true;
    }

    Ok(ResetResolution {
        reset_requested: consumed_agent,
        consumed_agent,
    })
}

pub fn write_file_backed_prompt(
    workspace: &Path,
    request_id: &str,
    prompt: &str,
    context: &str,
) -> Result<PromptArtifacts, ProviderError> {
    let prompt_dir = workspace.join("provider_prompts");
    fs::create_dir_all(&prompt_dir).map_err(|err| io_error(&prompt_dir, err))?;

    let prompt_file = prompt_dir.join(format!("{}_prompt.md", request_id));
    let context_file = prompt_dir.join(format!("{}_context.md", request_id));

    fs::write(&prompt_file, prompt).map_err(|err| io_error(&prompt_file, err))?;
    fs::write(&context_file, context).map_err(|err| io_error(&context_file, err))?;

    Ok(PromptArtifacts {
        prompt_file,
        context_files: vec![context_file],
    })
}

pub fn read_to_string(path: &Path) -> Result<String, ProviderError> {
    let mut file = fs::File::open(path).map_err(|err| io_error(path, err))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|err| io_error(path, err))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_prompt_artifacts(base: &Path) -> PromptArtifacts {
        write_file_backed_prompt(base, "req-1", "prompt", "ctx").expect("prompt artifacts")
    }

    fn sample_request(provider: ProviderKind, base: &Path) -> ProviderRequest {
        ProviderRequest {
            agent_id: "agent-1".to_string(),
            provider,
            model: "sonnet".to_string(),
            cwd: base.to_path_buf(),
            message: "use files".to_string(),
            prompt_artifacts: sample_prompt_artifacts(base),
            timeout: Duration::from_secs(1),
            reset_requested: false,
            fresh_on_failure: false,
            env_overrides: BTreeMap::new(),
        }
    }

    #[test]
    fn anthropic_model_aliases_map() {
        assert_eq!(
            resolve_anthropic_model("sonnet").expect("map"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            resolve_anthropic_model("opus").expect("map"),
            "claude-opus-4-6"
        );
        assert!(resolve_anthropic_model("haiku").is_err());
    }

    #[test]
    fn invocation_builds_expected_anthropic_args() {
        let dir = tempdir().expect("tempdir");
        let req = sample_request(ProviderKind::Anthropic, dir.path());
        let spec = build_invocation(&req, &RunnerBinaries::default()).expect("build");
        assert_eq!(spec.binary, "claude");
        assert!(spec
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));
        assert!(spec.args.contains(&"-c".to_string()));
        assert!(spec.args.contains(&"-p".to_string()));
    }

    #[test]
    fn invocation_builds_expected_openai_args_and_resume_behavior() {
        let dir = tempdir().expect("tempdir");
        let mut req = sample_request(ProviderKind::OpenAi, dir.path());
        req.model = "gpt-5.2".to_string();

        let spec = build_invocation(&req, &RunnerBinaries::default()).expect("build");
        assert_eq!(spec.binary, "codex");
        assert_eq!(&spec.args[0], "exec");
        assert!(spec.args.contains(&"resume".to_string()));
        assert!(spec.args.contains(&"--json".to_string()));

        req.reset_requested = true;
        let reset_spec = build_invocation(&req, &RunnerBinaries::default()).expect("build reset");
        assert!(!reset_spec.args.contains(&"resume".to_string()));
    }

    #[test]
    fn openai_jsonl_parser_reads_last_completed_agent_message() {
        let data = r#"
{"type":"item.completed","item":{"type":"agent_message","text":"first"}}
{"type":"item.completed","item":{"type":"agent_message","content":[{"text":"second"}]}}
"#;

        let parsed = parse_openai_jsonl(data).expect("parsed");
        assert_eq!(parsed, "second");
    }

    #[test]
    fn reset_flags_are_consumed_once() {
        let dir = tempdir().expect("tempdir");
        let agent = dir.path().join("agent/reset_flag");
        fs::create_dir_all(agent.parent().expect("parent")).expect("create parent");
        fs::write(&agent, "1").expect("write agent");

        let first = consume_reset_flag(&agent).expect("consume");
        assert!(first.reset_requested);
        assert!(first.consumed_agent);
        assert!(!agent.exists());

        let second = consume_reset_flag(&agent).expect("consume again");
        assert!(!second.reset_requested);
    }
}
