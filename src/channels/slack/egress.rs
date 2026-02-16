use super::{io_error, json_error, SlackError, SlackProfileRuntime};
use crate::queue::{sorted_outgoing_paths, OutgoingMessage, QueuePaths};
use std::collections::BTreeMap;
use std::fs;

const OUTBOUND_CHUNK_CHARS: usize = 3500;

fn parse_conversation_id(value: &str) -> Result<(&str, &str), SlackError> {
    let (channel_id, thread_ts) = value
        .split_once(':')
        .ok_or_else(|| SlackError::InvalidConversationId(value.to_string()))?;
    if channel_id.trim().is_empty() || thread_ts.trim().is_empty() {
        return Err(SlackError::InvalidConversationId(value.to_string()));
    }
    Ok((channel_id, thread_ts))
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

fn resolve_outgoing_profile_id(
    outgoing: &OutgoingMessage,
    available_profiles: &BTreeMap<String, SlackProfileRuntime>,
) -> Result<String, SlackError> {
    if let Some(profile_id) = outgoing
        .channel_profile_id
        .as_ref()
        .filter(|id| !id.trim().is_empty())
    {
        if available_profiles.contains_key(profile_id) {
            return Ok(profile_id.clone());
        }
        return Err(SlackError::UnknownChannelProfile(profile_id.clone()));
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

        let profile_id = resolve_outgoing_profile_id(&outgoing, runtimes)?;
        let runtime = runtimes
            .get(&profile_id)
            .ok_or_else(|| SlackError::UnknownChannelProfile(profile_id.clone()))?;

        let conversation_id = outgoing.conversation_id.as_deref().ok_or_else(|| {
            SlackError::InvalidConversationId("missing conversation_id".to_string())
        })?;
        let (channel_id, thread_ts) = parse_conversation_id(conversation_id)?;

        for chunk in chunk_message(&outgoing.message) {
            runtime
                .api
                .post_message(channel_id, Some(thread_ts), &chunk)
                .map_err(|err| SlackError::OutboundDelivery {
                    message_id: outgoing.message_id.clone(),
                    profile_id: profile_id.clone(),
                    reason: err.to_string(),
                })?;
        }

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
        assert_eq!(parsed.0, "C123");
        assert_eq!(parsed.1, "1700.00");

        assert!(parse_conversation_id("C123").is_err());
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
