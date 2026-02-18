use crate::config::{ChannelKind, Settings};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlackPostingMode {
    ChannelPost,
    ThreadReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackTargetRef {
    pub channel_profile_id: String,
    pub channel_id: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
    pub posting_mode: SlackPostingMode,
}

pub fn parse_slack_target_ref(
    value: &Value,
    field_path: &str,
) -> Result<Option<SlackTargetRef>, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{field_path} must be a JSON object"))?;

    let Some(channel) = obj.get("channel").and_then(Value::as_str) else {
        return Ok(None);
    };
    let channel = channel.trim();
    if channel != "slack" {
        return Err(format!(
            "{field_path}.channel `{channel}` is not supported; expected `slack`"
        ));
    }

    let channel_profile_id = required_trimmed_string(obj, "channelProfileId", field_path)?;
    let channel_id = required_trimmed_string(obj, "channelId", field_path)?;
    let posting_mode_raw = required_trimmed_string(obj, "postingMode", field_path)?;
    let posting_mode = parse_posting_mode(&posting_mode_raw, field_path)?;
    let thread_ts = optional_trimmed_string(obj, "threadTs", field_path)?;

    let thread_ts = match posting_mode {
        SlackPostingMode::ChannelPost => None,
        SlackPostingMode::ThreadReply => {
            let value = thread_ts.ok_or_else(|| {
                format!("{field_path}.threadTs is required when postingMode=thread_reply")
            })?;
            Some(value)
        }
    };

    Ok(Some(SlackTargetRef {
        channel_profile_id,
        channel_id,
        thread_ts,
        posting_mode,
    }))
}

pub fn slack_target_ref_to_value(target: &SlackTargetRef) -> Value {
    let mut obj = Map::from_iter([
        ("channel".to_string(), Value::String("slack".to_string())),
        (
            "channelProfileId".to_string(),
            Value::String(target.channel_profile_id.clone()),
        ),
        (
            "channelId".to_string(),
            Value::String(target.channel_id.clone()),
        ),
        (
            "postingMode".to_string(),
            Value::String(
                match target.posting_mode {
                    SlackPostingMode::ChannelPost => "channel_post",
                    SlackPostingMode::ThreadReply => "thread_reply",
                }
                .to_string(),
            ),
        ),
    ]);
    if let Some(thread_ts) = target.thread_ts.as_ref() {
        obj.insert("threadTs".to_string(), Value::String(thread_ts.clone()));
    }
    Value::Object(obj)
}

pub fn validate_profile_mapping(
    settings: &Settings,
    orchestrator_id: &str,
    target: Option<&SlackTargetRef>,
) -> Result<(), String> {
    let Some(target) = target else {
        return Ok(());
    };
    let profile = settings
        .channel_profiles
        .get(&target.channel_profile_id)
        .ok_or_else(|| {
            format!(
                "targetRef.channelProfileId `{}` is not configured",
                target.channel_profile_id
            )
        })?;
    if profile.channel != ChannelKind::Slack {
        return Err(format!(
            "targetRef.channelProfileId `{}` must reference a slack channel profile",
            target.channel_profile_id
        ));
    }
    if profile.orchestrator_id != orchestrator_id {
        return Err(format!(
            "targetRef.channelProfileId `{}` maps to orchestrator `{}` (expected `{}`)",
            target.channel_profile_id, profile.orchestrator_id, orchestrator_id
        ));
    }
    Ok(())
}

pub fn slack_target_from_conversation(
    channel_profile_id: &str,
    conversation_id: &str,
) -> Result<SlackTargetRef, String> {
    let channel_profile_id = channel_profile_id.trim();
    if channel_profile_id.is_empty() {
        return Err("channelProfileId must be non-empty".to_string());
    }
    let conversation_id = conversation_id.trim();
    if conversation_id.is_empty() {
        return Err("conversationId must be non-empty".to_string());
    }

    if let Some((channel_id, thread_ts)) = conversation_id.split_once(':') {
        let channel_id = channel_id.trim();
        let thread_ts = thread_ts.trim();
        if channel_id.is_empty() || thread_ts.is_empty() {
            return Err(format!("invalid conversation id `{conversation_id}`"));
        }
        return Ok(SlackTargetRef {
            channel_profile_id: channel_profile_id.to_string(),
            channel_id: channel_id.to_string(),
            thread_ts: Some(thread_ts.to_string()),
            posting_mode: SlackPostingMode::ThreadReply,
        });
    }

    Ok(SlackTargetRef {
        channel_profile_id: channel_profile_id.to_string(),
        channel_id: conversation_id.to_string(),
        thread_ts: None,
        posting_mode: SlackPostingMode::ChannelPost,
    })
}

fn required_trimmed_string(
    obj: &Map<String, Value>,
    key: &str,
    field_path: &str,
) -> Result<String, String> {
    let value = obj
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{field_path}.{key} is required"))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_path}.{key} must be non-empty"));
    }
    Ok(trimmed.to_string())
}

fn optional_trimmed_string(
    obj: &Map<String, Value>,
    key: &str,
    field_path: &str,
) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Err(format!(
                    "{field_path}.{key} must be non-empty when provided"
                ))
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Some(_) => Err(format!("{field_path}.{key} must be a string")),
    }
}

fn parse_posting_mode(value: &str, field_path: &str) -> Result<SlackPostingMode, String> {
    match value {
        "channel_post" => Ok(SlackPostingMode::ChannelPost),
        "thread_reply" => Ok(SlackPostingMode::ThreadReply),
        other => Err(format!(
            "{field_path}.postingMode must be channel_post|thread_reply (got `{other}`)"
        )),
    }
}
