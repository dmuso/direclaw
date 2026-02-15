use super::api::SlackMessage;
use super::cursor_store::{load_cursor_state, save_cursor_state};
use super::{io_error, json_error, now_secs, sanitize_component, SlackError, SlackProfileRuntime};
use crate::config::ChannelProfile;
use crate::queue::QueuePaths;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub fn should_accept_channel_message(
    profile: &ChannelProfile,
    allowlist: &BTreeSet<String>,
    conversation_id: &str,
    message_text: &str,
    message_ts: &str,
    thread_ts: Option<&str>,
) -> bool {
    let in_thread = thread_ts
        .map(|thread| thread != message_ts)
        .unwrap_or(false);
    let allowlisted = allowlist.contains(conversation_id);
    let mentioned = profile
        .slack_app_user_id
        .as_ref()
        .map(|id| message_text.contains(&format!("<@{id}>")))
        .unwrap_or(false);
    let _mentions_required = profile.require_mention_in_channels.unwrap_or(false);
    in_thread || allowlisted || mentioned
}

fn enqueue_incoming(
    queue_paths: &QueuePaths,
    profile_id: &str,
    conversation_id: &str,
    message: &SlackMessage,
) -> Result<bool, SlackError> {
    let sender_id = message
        .user
        .as_ref()
        .filter(|v| !v.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let ts = message.ts.clone();
    let message_id = format!(
        "slack-{}-{}-{}",
        sanitize_component(profile_id),
        sanitize_component(conversation_id),
        sanitize_component(&ts)
    );
    let path = queue_paths.incoming.join(format!("{message_id}.json"));
    if path.exists() {
        return Ok(false);
    }
    let thread_ts = message.thread_ts.clone().unwrap_or_else(|| ts.clone());

    let payload = crate::queue::IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some(profile_id.to_string()),
        sender: sender_id.clone(),
        sender_id,
        message: message.text.clone().unwrap_or_default(),
        timestamp: now_secs(),
        message_id,
        conversation_id: Some(format!("{conversation_id}:{thread_ts}")),
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    let body = serde_json::to_vec_pretty(&payload).map_err(|e| json_error(&path, e))?;
    fs::write(&path, body).map_err(|e| io_error(&path, e))?;
    Ok(true)
}

pub(super) fn process_inbound_for_profile(
    state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
) -> Result<usize, SlackError> {
    let mut cursor_state = load_cursor_state(state_root, profile_id)?;
    let mut enqueued = 0usize;

    for conversation in runtime.api.list_conversations()? {
        let oldest = cursor_state
            .conversations
            .get(&conversation.id)
            .map(String::as_str);
        let mut latest_ts = oldest.unwrap_or("0").to_string();
        let messages = runtime.api.conversation_history(&conversation.id, oldest)?;
        for message in messages {
            if message.ts.trim().is_empty() {
                continue;
            }
            if message.user.is_none() {
                continue;
            }
            if message.bot_id.is_some() || message.subtype.is_some() {
                continue;
            }

            if !conversation.is_im
                && !should_accept_channel_message(
                    &runtime.profile,
                    &runtime.allowlist,
                    &conversation.id,
                    message.text.as_deref().unwrap_or(""),
                    &message.ts,
                    message.thread_ts.as_deref(),
                )
            {
                continue;
            }

            if enqueue_incoming(queue_paths, profile_id, &conversation.id, &message)? {
                enqueued += 1;
            }

            if message.ts > latest_ts {
                latest_ts = message.ts;
            }
        }
        cursor_state
            .conversations
            .insert(conversation.id.clone(), latest_ts);
    }

    save_cursor_state(state_root, profile_id, &cursor_state)?;
    Ok(enqueued)
}
