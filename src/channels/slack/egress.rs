use super::{io_error, json_error, SlackError, SlackProfileRuntime};
use crate::orchestration::slack_target::{
    parse_slack_target_ref, SlackPostingMode, SlackTargetRef,
};
use crate::queue::{sorted_outgoing_paths, OutgoingMessage, QueuePaths};
use std::collections::BTreeMap;
use std::fs;

const OUTBOUND_CHUNK_CHARS: usize = 3500;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeliveryTarget {
    channel_id: String,
    thread_ts: Option<String>,
}

fn parse_conversation_id(value: &str) -> Result<DeliveryTarget, SlackError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(SlackError::InvalidConversationId(value.to_string()));
    }

    if let Some((channel_id, thread_ts)) = trimmed.split_once(':') {
        let channel_id = channel_id.trim();
        let thread_ts = thread_ts.trim();
        if channel_id.is_empty() || thread_ts.is_empty() {
            return Err(SlackError::InvalidConversationId(value.to_string()));
        }
        return Ok(DeliveryTarget {
            channel_id: channel_id.to_string(),
            thread_ts: Some(thread_ts.to_string()),
        });
    }

    Ok(DeliveryTarget {
        channel_id: trimmed.to_string(),
        thread_ts: None,
    })
}

fn chunk_message(input: &str) -> Vec<String> {
    if input.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for ch in input.chars() {
        if count >= OUTBOUND_CHUNK_CHARS {
            out.push(current);
            current = String::new();
            count = 0;
        }
        current.push(ch);
        count += 1;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn parse_outgoing_target_ref(
    outgoing: &OutgoingMessage,
) -> Result<Option<SlackTargetRef>, SlackError> {
    let Some(target_ref) = outgoing.target_ref.as_ref() else {
        return Ok(None);
    };
    parse_slack_target_ref(target_ref, "targetRef").map_err(|reason| SlackError::InvalidTargetRef {
        message_id: outgoing.message_id.clone(),
        reason,
    })
}

fn resolve_outgoing_profile_id(
    outgoing: &OutgoingMessage,
    target_ref: Option<&SlackTargetRef>,
    available_profiles: &BTreeMap<String, SlackProfileRuntime>,
) -> Result<String, SlackError> {
    if let Some(profile_id) = outgoing
        .channel_profile_id
        .as_ref()
        .filter(|id| !id.trim().is_empty())
    {
        if let Some(target) = target_ref {
            if target.channel_profile_id != *profile_id {
                return Err(SlackError::InvalidTargetRef {
                    message_id: outgoing.message_id.clone(),
                    reason: format!(
                        "channel_profile_id `{profile_id}` must match targetRef.channelProfileId `{}`",
                        target.channel_profile_id
                    ),
                });
            }
        }
        if available_profiles.contains_key(profile_id) {
            return Ok(profile_id.clone());
        }
        return Err(SlackError::UnknownChannelProfile(profile_id.clone()));
    }
    if let Some(target) = target_ref {
        if available_profiles.contains_key(&target.channel_profile_id) {
            return Ok(target.channel_profile_id.clone());
        }
        return Err(SlackError::UnknownChannelProfile(
            target.channel_profile_id.clone(),
        ));
    }
    if available_profiles.len() == 1 {
        return Ok(available_profiles
            .keys()
            .next()
            .cloned()
            .expect("len checked"));
    }
    Err(SlackError::MissingChannelProfileId {
        message_id: outgoing.message_id.clone(),
    })
}

fn resolve_delivery_target(
    outgoing: &OutgoingMessage,
    target_ref: Option<&SlackTargetRef>,
) -> Result<DeliveryTarget, SlackError> {
    if let Some(target) = target_ref {
        return Ok(delivery_target_from_target_ref(target));
    }

    let conversation_id = outgoing
        .conversation_id
        .as_deref()
        .ok_or_else(|| SlackError::InvalidConversationId("missing conversation_id".to_string()))?;
    parse_conversation_id(conversation_id)
}

fn delivery_target_from_target_ref(target: &SlackTargetRef) -> DeliveryTarget {
    DeliveryTarget {
        channel_id: target.channel_id.clone(),
        thread_ts: target.thread_ts.clone(),
    }
}

fn enforce_channel_policy(
    outgoing: &OutgoingMessage,
    profile_id: &str,
    target: &DeliveryTarget,
    runtime: &SlackProfileRuntime,
    target_ref: Option<&SlackTargetRef>,
) -> Result<(), SlackError> {
    let enforce_allowlist = matches!(
        target_ref.map(|target| target.posting_mode),
        Some(SlackPostingMode::ChannelPost)
    );
    if !enforce_allowlist {
        return Ok(());
    }
    let is_dm = target.channel_id.starts_with('D');
    if !is_dm
        && !runtime.allowlist.is_empty()
        && !runtime.allowlist.contains(target.channel_id.as_str())
    {
        return Err(SlackError::UnauthorizedChannelTarget {
            message_id: outgoing.message_id.clone(),
            profile_id: profile_id.to_string(),
            channel_id: target.channel_id.clone(),
        });
    }
    Ok(())
}

fn deliver_targeted_post(
    outgoing: &OutgoingMessage,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
    target: &DeliveryTarget,
    target_ref: Option<&SlackTargetRef>,
) -> Result<(), SlackError> {
    enforce_channel_policy(outgoing, profile_id, target, runtime, target_ref)?;
    for chunk in chunk_message(&outgoing.message) {
        runtime
            .api
            .post_message(&target.channel_id, target.thread_ts.as_deref(), &chunk)
            .map_err(|err| SlackError::OutboundDelivery {
                message_id: outgoing.message_id.clone(),
                profile_id: profile_id.to_string(),
                reason: err.to_string(),
            })?;
    }
    Ok(())
}

pub(super) fn process_outbound(
    queue_paths: &QueuePaths,
    runtimes: &BTreeMap<String, SlackProfileRuntime>,
) -> Result<usize, SlackError> {
    let mut sent = 0usize;

    for path in
        sorted_outgoing_paths(queue_paths).map_err(|e| io_error(&queue_paths.outgoing, e))?
    {
        let raw = fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
        let outgoing: OutgoingMessage =
            serde_json::from_str(&raw).map_err(|e| json_error(&path, e))?;
        if outgoing.channel != "slack" {
            continue;
        }
        let target_ref = parse_outgoing_target_ref(&outgoing)?;

        let profile_id = resolve_outgoing_profile_id(&outgoing, target_ref.as_ref(), runtimes)?;
        let runtime = runtimes
            .get(&profile_id)
            .ok_or_else(|| SlackError::UnknownChannelProfile(profile_id.clone()))?;

        let target = resolve_delivery_target(&outgoing, target_ref.as_ref())?;
        deliver_targeted_post(
            &outgoing,
            &profile_id,
            runtime,
            &target,
            target_ref.as_ref(),
        )?;

        fs::remove_file(&path).map_err(|e| io_error(&path, e))?;
        sent += 1;
    }

    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_parser_requires_channel_and_thread() {
        let parsed = parse_conversation_id("C123:1700.00").expect("parsed");
        assert_eq!(parsed.channel_id, "C123");
        assert_eq!(parsed.thread_ts.as_deref(), Some("1700.00"));

        let channel_only = parse_conversation_id("C123").expect("channel post");
        assert_eq!(channel_only.channel_id, "C123");
        assert!(channel_only.thread_ts.is_none());

        assert!(parse_conversation_id(":1700").is_err());
    }

    #[test]
    fn message_chunking_uses_expected_limit() {
        let input = "x".repeat(OUTBOUND_CHUNK_CHARS + 2);
        let chunks = chunk_message(&input);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), OUTBOUND_CHUNK_CHARS);
        assert_eq!(chunks[1].chars().count(), 2);
    }
}
