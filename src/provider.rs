use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::BufReader;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub mod invocation;
pub mod model_map;

pub use invocation::build_invocation;
pub use model_map::resolve_anthropic_model;

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

#[derive(Debug, Clone)]
pub struct RunnerBinaries {
    pub anthropic: String,
    pub openai: String,
}

impl Default for RunnerBinaries {
    fn default() -> Self {
        Self {
            anthropic: "claude".to_string(),
            openai: "codex".to_string(),
        }
    }
}

pub(crate) fn io_error(path: &Path, source: std::io::Error) -> ProviderError {
    ProviderError::Io {
        path: path.display().to_string(),
        source,
    }
}

pub fn run_provider(
    request: &ProviderRequest,
    binaries: &RunnerBinaries,
) -> Result<ProviderResult, ProviderError> {
    let spec = build_invocation(request, binaries)?;

    let command_form = format!("{} {}", spec.binary, spec.args.join(" "));
    let base_log = InvocationLog {
        agent_id: request.agent_id.clone(),
        provider: request.provider.clone(),
        model: spec.resolved_model.clone(),
        command_form,
        working_directory: request.cwd.clone(),
        prompt_file: request.prompt_artifacts.prompt_file.clone(),
        context_files: request.prompt_artifacts.context_files.clone(),
        exit_code: None,
        timed_out: false,
    };

    let mut command = Command::new(&spec.binary);
    command
        .current_dir(&request.cwd)
        .args(&spec.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (k, v) in &request.env_overrides {
        command.env(k, v);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(ProviderError::MissingBinary {
                provider: request.provider.clone(),
                binary: spec.binary,
                log: Box::new(base_log),
            })
        }
        Err(err) => return Err(io_error(&request.cwd, err)),
    };

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io_error(&request.cwd, std::io::Error::other("missing stdout pipe")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io_error(&request.cwd, std::io::Error::other("missing stderr pipe")))?;

    let stdout_reader = thread::spawn(move || {
        let mut buf = String::new();
        let mut reader = BufReader::new(stdout);
        let _ = reader.read_to_string(&mut buf);
        buf
    });
    let stderr_reader = thread::spawn(move || {
        let mut buf = String::new();
        let mut reader = BufReader::new(stderr);
        let _ = reader.read_to_string(&mut buf);
        buf
    });

    let start = Instant::now();
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() > request.timeout {
                    let _ = child.kill();
                    let status = child.wait().map_err(|e| io_error(&request.cwd, e))?;
                    let _stdout = stdout_reader.join().unwrap_or_default();
                    let _stderr = stderr_reader.join().unwrap_or_default();
                    let mut log = base_log.clone();
                    log.timed_out = true;
                    log.exit_code = status.code();
                    return Err(ProviderError::Timeout {
                        provider: request.provider.clone(),
                        timeout_ms: request.timeout.as_millis() as u64,
                        log: Box::new(log),
                    });
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) => return Err(io_error(&request.cwd, err)),
        }
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();

    if !exit_status.success() {
        let mut log = base_log.clone();
        log.exit_code = exit_status.code();
        return Err(ProviderError::NonZeroExit {
            provider: request.provider.clone(),
            exit_code: exit_status.code().unwrap_or(-1),
            stderr,
            log: Box::new(log),
        });
    }

    let mut parse_log = base_log.clone();
    parse_log.exit_code = exit_status.code();
    let message_result = match request.provider {
        ProviderKind::Anthropic => parse_anthropic_output(&stdout),
        ProviderKind::OpenAi => parse_openai_jsonl(&stdout),
    };
    let message = message_result.map_err(|err| match err {
        ProviderError::ParseFailure {
            provider, reason, ..
        } => ProviderError::ParseFailure {
            provider,
            reason,
            log: Some(Box::new(parse_log.clone())),
        },
        other => other,
    })?;

    Ok(ProviderResult {
        message,
        log: parse_log,
    })
}

fn parse_anthropic_output(stdout: &str) -> Result<String, ProviderError> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(ProviderError::ParseFailure {
            provider: ProviderKind::Anthropic,
            reason: "stdout was empty".to_string(),
            log: None,
        });
    }
    Ok(trimmed.to_string())
}

fn extract_agent_message(item: &Value) -> Option<String> {
    if let Some(text) = item.get("text").and_then(Value::as_str) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(message) = item.get("message").and_then(Value::as_str) {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(content) = item.get("content") {
        if let Some(content_string) = content.as_str() {
            let trimmed = content_string.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }

        if let Some(arr) = content.as_array() {
            let mut lines = Vec::new();
            for entry in arr {
                if let Some(text) = entry.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        lines.push(trimmed.to_string());
                    }
                }
            }
            if !lines.is_empty() {
                return Some(lines.join("\n"));
            }
        }
    }

    None
}

pub fn parse_openai_jsonl(stdout: &str) -> Result<String, ProviderError> {
    let mut last_message = None;

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let value: Value =
            serde_json::from_str(line).map_err(|err| ProviderError::ParseFailure {
                provider: ProviderKind::OpenAi,
                reason: format!("invalid jsonl event: {err}"),
                log: None,
            })?;

        if value.get("type").and_then(Value::as_str) != Some("item.completed") {
            continue;
        }

        let Some(item) = value.get("item") else {
            continue;
        };
        if item.get("type").and_then(Value::as_str) != Some("agent_message") {
            continue;
        }

        if let Some(message) = extract_agent_message(item) {
            last_message = Some(message);
        }
    }

    last_message.ok_or_else(|| ProviderError::ParseFailure {
        provider: ProviderKind::OpenAi,
        reason: "missing terminal agent_message item.completed event".to_string(),
        log: None,
    })
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
