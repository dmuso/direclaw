use crate::provider::{ProviderError, ProviderKind};
use serde_json::Value;

pub(crate) fn parse_anthropic_output(stdout: &str) -> Result<String, ProviderError> {
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
