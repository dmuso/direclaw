use crate::provider::invocation::build_invocation;
use crate::provider::output_parse::parse_anthropic_output;
use crate::provider::{
    io_error, parse_openai_jsonl, InvocationLog, ProviderError, ProviderKind, ProviderRequest,
    ProviderResult,
};
use std::io::BufReader;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
