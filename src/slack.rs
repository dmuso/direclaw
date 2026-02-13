use crate::config::{ChannelProfile, Settings};
use crate::queue::{OutgoingMessage, QueuePaths};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_SLACK_API_BASE: &str = "https://slack.com/api";
const OUTBOUND_CHUNK_CHARS: usize = 3500;

#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    #[error("slack channel is disabled in settings")]
    ChannelDisabled,
    #[error("no slack channel profiles are configured")]
    NoSlackProfiles,
    #[error("missing required env var `{0}`")]
    MissingEnvVar(String),
    #[error("invalid conversation id `{0}` for slack outgoing message")]
    InvalidConversationId(String),
    #[error("unknown slack channel profile `{0}` in outgoing message")]
    UnknownChannelProfile(String),
    #[error("outgoing slack message `{message_id}` has no channel_profile_id and multiple slack profiles exist")]
    MissingChannelProfileId { message_id: String },
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

#[derive(Debug, Clone)]
struct SlackProfileRuntime {
    profile: ChannelProfile,
    api: SlackApiClient,
    allowlist: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct SlackApiClient {
    api_base: String,
    bot_token: String,
    app_token: String,
}

#[derive(Debug, Clone)]
struct EnvConfig {
    bot_token: String,
    app_token: String,
    allowlist: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct SlackCursorState {
    conversations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEnvelope<T> {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(flatten)]
    data: T,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct EmptyData {}

#[derive(Debug, Clone, Deserialize)]
struct OpenConnectionData {
    #[allow(dead_code)]
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ConversationsListData {
    conversations: Vec<ConversationSummary>,
    #[serde(default)]
    response_metadata: ResponseMetadata,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ResponseMetadata {
    #[serde(default)]
    next_cursor: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ConversationSummary {
    id: String,
    #[serde(default)]
    is_im: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ConversationsHistoryData {
    #[serde(default)]
    messages: Vec<SlackMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackMessage {
    #[serde(default)]
    ts: String,
    #[serde(default)]
    thread_ts: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    bot_id: Option<String>,
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

fn profile_env_key(prefix: &str, profile_id: &str) -> String {
    let mapped: String = profile_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix}_{mapped}")
}

fn env_var_fallback(profile_key: &str, global_key: &str) -> Option<String> {
    std::env::var(profile_key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var(global_key)
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
}

fn parse_allowlist(value: Option<String>) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    if let Some(value) = value {
        for part in value.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                result.insert(trimmed.to_string());
            }
        }
    }
    result
}

fn load_env_config(profile_id: &str) -> Result<EnvConfig, SlackError> {
    let bot_profile = profile_env_key("SLACK_BOT_TOKEN", profile_id);
    let app_profile = profile_env_key("SLACK_APP_TOKEN", profile_id);
    let allowlist_profile = profile_env_key("SLACK_CHANNEL_ALLOWLIST", profile_id);

    let bot_token = env_var_fallback(&bot_profile, "SLACK_BOT_TOKEN")
        .ok_or_else(|| SlackError::MissingEnvVar("SLACK_BOT_TOKEN".to_string()))?;
    let app_token = env_var_fallback(&app_profile, "SLACK_APP_TOKEN")
        .ok_or_else(|| SlackError::MissingEnvVar("SLACK_APP_TOKEN".to_string()))?;
    let allowlist = parse_allowlist(env_var_fallback(
        &allowlist_profile,
        "SLACK_CHANNEL_ALLOWLIST",
    ));

    Ok(EnvConfig {
        bot_token,
        app_token,
        allowlist,
    })
}

impl SlackApiClient {
    fn new(bot_token: String, app_token: String) -> Self {
        let api_base = std::env::var("DIRECLAW_SLACK_API_BASE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SLACK_API_BASE.to_string());
        Self {
            api_base,
            bot_token,
            app_token,
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.api_base.trim_end_matches('/'), path)
    }

    fn get_with_token<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, String)],
        token: &str,
    ) -> Result<T, SlackError> {
        let mut url = self.endpoint(path);
        if !query.is_empty() {
            let encoded = query
                .iter()
                .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            url = format!("{url}?{encoded}");
        }

        let response = ureq::get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|e| SlackError::ApiRequest(e.to_string()))?;

        response
            .into_json::<T>()
            .map_err(|e| SlackError::ApiRequest(e.to_string()))
    }

    fn post_json_with_token<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
        token: &str,
    ) -> Result<T, SlackError> {
        let url = self.endpoint(path);
        let response = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(
                serde_json::to_value(body).map_err(|e| SlackError::ApiRequest(e.to_string()))?,
            )
            .map_err(|e| SlackError::ApiRequest(e.to_string()))?;

        response
            .into_json::<T>()
            .map_err(|e| SlackError::ApiRequest(e.to_string()))
    }

    fn validate_connection(&self) -> Result<(), SlackError> {
        let auth: SlackEnvelope<EmptyData> =
            self.get_with_token("auth.test", &[], &self.bot_token)?;
        if !auth.ok {
            return Err(SlackError::ApiResponse(
                auth.error.unwrap_or_else(|| "auth.test failed".to_string()),
            ));
        }

        let conn: SlackEnvelope<OpenConnectionData> =
            self.post_json_with_token("apps.connections.open", &json!({}), &self.app_token)?;
        if !conn.ok {
            return Err(SlackError::ApiResponse(
                conn.error
                    .unwrap_or_else(|| "apps.connections.open failed".to_string()),
            ));
        }

        Ok(())
    }

    fn list_conversations(&self) -> Result<Vec<ConversationSummary>, SlackError> {
        let mut all = Vec::new();
        let mut cursor = String::new();
        loop {
            let mut query = vec![
                ("types", "im,public_channel,private_channel".to_string()),
                ("limit", "200".to_string()),
            ];
            if !cursor.is_empty() {
                query.push(("cursor", cursor.clone()));
            }

            let envelope: SlackEnvelope<ConversationsListData> =
                self.get_with_token("conversations.list", &query, &self.bot_token)?;
            if !envelope.ok {
                return Err(SlackError::ApiResponse(
                    envelope
                        .error
                        .unwrap_or_else(|| "conversations.list failed".to_string()),
                ));
            }
            let data = envelope.data;
            all.extend(data.conversations);
            cursor = data.response_metadata.next_cursor;
            if cursor.trim().is_empty() {
                break;
            }
        }
        Ok(all)
    }

    fn conversation_history(
        &self,
        conversation_id: &str,
        oldest: Option<&str>,
    ) -> Result<Vec<SlackMessage>, SlackError> {
        let mut query = vec![
            ("channel", conversation_id.to_string()),
            ("inclusive", "false".to_string()),
            ("limit", "200".to_string()),
        ];
        if let Some(oldest) = oldest {
            if !oldest.trim().is_empty() {
                query.push(("oldest", oldest.to_string()));
            }
        }
        let envelope: SlackEnvelope<ConversationsHistoryData> =
            self.get_with_token("conversations.history", &query, &self.bot_token)?;
        if !envelope.ok {
            return Err(SlackError::ApiResponse(
                envelope
                    .error
                    .unwrap_or_else(|| "conversations.history failed".to_string()),
            ));
        }
        Ok(envelope.data.messages)
    }

    fn post_message(
        &self,
        channel_id: &str,
        thread_ts: Option<&str>,
        message: &str,
    ) -> Result<(), SlackError> {
        let mut body = json!({
            "channel": channel_id,
            "text": message,
        });
        if let Some(thread_ts) = thread_ts.filter(|v| !v.trim().is_empty()) {
            body["thread_ts"] = json!(thread_ts);
        }
        let envelope: SlackEnvelope<serde_json::Value> =
            self.post_json_with_token("chat.postMessage", &body, &self.bot_token)?;
        if !envelope.ok {
            return Err(SlackError::ApiResponse(
                envelope
                    .error
                    .unwrap_or_else(|| "chat.postMessage failed".to_string()),
            ));
        }
        Ok(())
    }
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

fn slack_profiles(settings: &Settings) -> BTreeMap<String, ChannelProfile> {
    settings
        .channel_profiles
        .iter()
        .filter_map(|(id, profile)| {
            if profile.channel == "slack" {
                Some((id.clone(), profile.clone()))
            } else {
                None
            }
        })
        .collect()
}

fn cursor_state_path(state_root: &Path, profile_id: &str) -> PathBuf {
    state_root
        .join("channels/slack")
        .join(sanitize_component(profile_id))
        .join("cursor.json")
}

fn load_cursor_state(state_root: &Path, profile_id: &str) -> Result<SlackCursorState, SlackError> {
    let path = cursor_state_path(state_root, profile_id);
    if !path.exists() {
        return Ok(SlackCursorState::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
    serde_json::from_str(&raw).map_err(|e| json_error(&path, e))
}

fn save_cursor_state(
    state_root: &Path,
    profile_id: &str,
    state: &SlackCursorState,
) -> Result<(), SlackError> {
    let path = cursor_state_path(state_root, profile_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(state).map_err(|e| json_error(&path, e))?;
    fs::write(&tmp, body).map_err(|e| io_error(&tmp, e))?;
    fs::rename(&tmp, &path).map_err(|e| io_error(&path, e))
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
    let mentions_required = profile.require_mention_in_channels.unwrap_or(false);

    if mentions_required {
        in_thread || allowlisted || mentioned
    } else {
        allowlisted || in_thread || mentioned || !text.trim().is_empty()
    }
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
        "slack-{}-{}",
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
                .post_message(channel_id, Some(thread_ts), &chunk)?;
        }

        fs::remove_file(&path).map_err(|e| io_error(&path, e))?;
        sent += 1;
    }

    Ok(sent)
}

pub fn sync_once(state_root: &Path, settings: &Settings) -> Result<SlackSyncReport, SlackError> {
    let slack_enabled = settings
        .channels
        .get("slack")
        .map(|cfg| cfg.enabled)
        .unwrap_or(false);
    if !slack_enabled {
        return Err(SlackError::ChannelDisabled);
    }

    let profiles = slack_profiles(settings);
    if profiles.is_empty() {
        return Err(SlackError::NoSlackProfiles);
    }

    let queue_paths = QueuePaths::from_state_root(state_root);
    fs::create_dir_all(&queue_paths.incoming).map_err(|e| io_error(&queue_paths.incoming, e))?;
    fs::create_dir_all(&queue_paths.outgoing).map_err(|e| io_error(&queue_paths.outgoing, e))?;

    let mut runtimes = BTreeMap::new();
    for (profile_id, profile) in profiles {
        let env = load_env_config(&profile_id)?;
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
        let mut state = SlackCursorState::default();
        state
            .conversations
            .insert("C123".to_string(), "1700000000.1".to_string());
        save_cursor_state(&state_root, "profile.main", &state).expect("save");
        let loaded = load_cursor_state(&state_root, "profile.main").expect("load");
        assert_eq!(loaded, state);
    }
}
