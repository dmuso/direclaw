use crate::config::{ChannelProfile, Settings};
use crate::queue::{OutgoingMessage, QueuePaths};
use api::{SlackApiClient, SlackMessage};
use auth::{configured_slack_allowlist, load_env_config, slack_profiles};
use cursor_store::{load_cursor_state, save_cursor_state};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const OUTBOUND_CHUNK_CHARS: usize = 3500;

pub mod api;
pub mod auth;
pub mod cursor_store;

pub use auth::{profile_credential_health, validate_startup_credentials};

#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    #[error("slack channel is disabled in settings")]
    ChannelDisabled,
    #[error("no slack channel profiles are configured")]
    NoSlackProfiles,
    #[error("missing required env var `{0}`")]
    MissingEnvVar(String),
    #[error("missing required env var `{key}` for slack profile `{profile_id}`")]
    MissingProfileScopedEnvVar { profile_id: String, key: String },
    #[error(
        "slack profiles `{profile_a}` and `{profile_b}` resolve to the same `{credential}` token; configure distinct profile-scoped credentials"
    )]
    DuplicateProfileCredential {
        credential: String,
        profile_a: String,
        profile_b: String,
    },
    #[error("invalid conversation id `{0}` for slack outgoing message")]
    InvalidConversationId(String),
    #[error("unknown slack channel profile `{0}` in outgoing message")]
    UnknownChannelProfile(String),
    #[error("outgoing slack message `{message_id}` has no channel_profile_id and multiple slack profiles exist")]
    MissingChannelProfileId { message_id: String },
    #[error(
        "failed to deliver outbound slack message `{message_id}` for profile `{profile_id}`: {reason}"
    )]
    OutboundDelivery {
        message_id: String,
        profile_id: String,
        reason: String,
    },
    #[error("slack api request failed: {0}")]
    ApiRequest(String),
    #[error("slack api responded with error `{0}`")]
    ApiResponse(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json error at {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlackSyncReport {
    pub profiles_processed: usize,
    pub inbound_enqueued: usize,
    pub outbound_messages_sent: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackProfileCredentialHealth {
    pub profile_id: String,
    pub ok: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
struct SlackProfileRuntime {
    profile: ChannelProfile,
    api: SlackApiClient,
    allowlist: BTreeSet<String>,
}

fn io_error(path: &Path, source: std::io::Error) -> SlackError {
    SlackError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn json_error(path: &Path, source: serde_json::Error) -> SlackError {
    SlackError::Json {
        path: path.display().to_string(),
        source,
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn sanitize_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn sorted_outgoing_paths(paths: &QueuePaths) -> Result<Vec<PathBuf>, SlackError> {
    let mut files = Vec::new();
    for entry in fs::read_dir(&paths.outgoing).map_err(|e| io_error(&paths.outgoing, e))? {
        let entry = entry.map_err(|e| io_error(&paths.outgoing, e))?;
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) == Some("json") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

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

fn should_accept_channel_message(
    profile: &ChannelProfile,
    allowlist: &BTreeSet<String>,
    conversation_id: &str,
    message: &SlackMessage,
) -> bool {
    let text = message.text.as_deref().unwrap_or("");
    let in_thread = message
        .thread_ts
        .as_ref()
        .map(|thread| thread != &message.ts)
        .unwrap_or(false);
    let allowlisted = allowlist.contains(conversation_id);
    let mentioned = profile
        .slack_app_user_id
        .as_ref()
        .map(|id| text.contains(&format!("<@{id}>")))
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

fn process_inbound_for_profile(
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
                    &message,
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

fn process_outbound(
    queue_paths: &QueuePaths,
    runtimes: &BTreeMap<String, SlackProfileRuntime>,
) -> Result<usize, SlackError> {
    let mut sent = 0usize;

    for path in sorted_outgoing_paths(queue_paths)? {
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

fn slack_channel_enabled(settings: &Settings) -> bool {
    settings
        .channels
        .get("slack")
        .map(|cfg| cfg.enabled)
        .unwrap_or(false)
}

pub fn sync_once(state_root: &Path, settings: &Settings) -> Result<SlackSyncReport, SlackError> {
    validate_startup_credentials(settings)?;
    let profiles = slack_profiles(settings);
    let profile_scoped_tokens_required = profiles.len() > 1;
    let config_allowlist = configured_slack_allowlist(settings);

    let queue_paths = QueuePaths::from_state_root(state_root);
    fs::create_dir_all(&queue_paths.incoming).map_err(|e| io_error(&queue_paths.incoming, e))?;
    fs::create_dir_all(&queue_paths.outgoing).map_err(|e| io_error(&queue_paths.outgoing, e))?;

    let mut bot_token_profile = BTreeMap::<String, String>::new();
    let mut app_token_profile = BTreeMap::<String, String>::new();
    let mut runtimes = BTreeMap::new();
    for (profile_id, profile) in profiles {
        let env = load_env_config(
            &profile_id,
            profile_scoped_tokens_required,
            &config_allowlist,
        )?;
        if let Some(existing) = bot_token_profile.insert(env.bot_token.clone(), profile_id.clone())
        {
            return Err(SlackError::DuplicateProfileCredential {
                credential: "bot".to_string(),
                profile_a: existing,
                profile_b: profile_id,
            });
        }
        if let Some(existing) = app_token_profile.insert(env.app_token.clone(), profile_id.clone())
        {
            return Err(SlackError::DuplicateProfileCredential {
                credential: "app".to_string(),
                profile_a: existing,
                profile_b: profile_id,
            });
        }
        let api = SlackApiClient::new(env.bot_token, env.app_token);
        api.validate_connection()?;
        runtimes.insert(
            profile_id,
            SlackProfileRuntime {
                profile,
                api,
                allowlist: env.allowlist,
            },
        );
    }

    let mut report = SlackSyncReport {
        profiles_processed: runtimes.len(),
        ..SlackSyncReport::default()
    };

    for (profile_id, runtime) in &runtimes {
        report.inbound_enqueued +=
            process_inbound_for_profile(state_root, &queue_paths, profile_id, runtime)?;
    }
    report.outbound_messages_sent = process_outbound(&queue_paths, &runtimes)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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

    #[test]
    fn cursor_state_round_trip_is_stable() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join(".direclaw");
        let mut state = cursor_store::SlackCursorState::default();
        state
            .conversations
            .insert("C123".to_string(), "1700000000.1".to_string());
        cursor_store::save_cursor_state(&state_root, "profile.main", &state).expect("save");
        let loaded = cursor_store::load_cursor_state(&state_root, "profile.main").expect("load");
        assert_eq!(loaded, state);
    }
}
