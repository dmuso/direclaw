use crate::provider::{InvocationLog, ProviderError};
use crate::shared::logging::append_orchestrator_log_line;
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn append_security_log(state_root: &Path, line: &str) {
    let _ = append_orchestrator_log_line(state_root, line);
}

pub fn provider_error_log(error: &ProviderError) -> Option<&InvocationLog> {
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

pub fn persist_provider_invocation_log(
    path_root: &Path,
    log: &InvocationLog,
) -> std::io::Result<()> {
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

pub fn persist_selector_invocation_log(
    state_root: &Path,
    selector_id: &str,
    attempt: u32,
    log: Option<&InvocationLog>,
    error: Option<&str>,
) {
    let mut payload = Map::new();
    payload.insert(
        "event".to_string(),
        Value::String("selector.invocation".to_string()),
    );
    payload.insert(
        "selectorId".to_string(),
        Value::String(selector_id.to_string()),
    );
    payload.insert("attempt".to_string(), Value::from(attempt));
    payload.insert("timestamp".to_string(), Value::from(now_secs()));
    payload.insert(
        "status".to_string(),
        Value::String(
            if error.is_some() {
                "failed"
            } else {
                "succeeded"
            }
            .to_string(),
        ),
    );
    if let Some(error) = error {
        payload.insert("error".to_string(), Value::String(error.to_string()));
    }

    if let Some(log) = log {
        payload.insert("agentId".to_string(), Value::String(log.agent_id.clone()));
        payload.insert(
            "provider".to_string(),
            Value::String(log.provider.to_string()),
        );
        payload.insert("model".to_string(), Value::String(log.model.clone()));
        payload.insert(
            "commandForm".to_string(),
            Value::String(log.command_form.clone()),
        );
        payload.insert(
            "workingDirectory".to_string(),
            Value::String(log.working_directory.display().to_string()),
        );
        payload.insert(
            "promptFile".to_string(),
            Value::String(log.prompt_file.display().to_string()),
        );
        payload.insert(
            "contextFiles".to_string(),
            Value::Array(
                log.context_files
                    .iter()
                    .map(|path| Value::String(path.display().to_string()))
                    .collect(),
            ),
        );
        payload.insert(
            "exitCode".to_string(),
            match log.exit_code {
                Some(value) => Value::from(value),
                None => Value::Null,
            },
        );
        payload.insert("timedOut".to_string(), Value::Bool(log.timed_out));
    }

    let line = match serde_json::to_string(&Value::Object(payload)) {
        Ok(line) => line,
        Err(_) => return,
    };
    let _ = append_orchestrator_log_line(state_root, &line);
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
