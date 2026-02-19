use super::api::{ConversationSummary, SlackApiClient, SlackMessage};
use super::cursor_store::{load_cursor_state, save_cursor_state};
use super::{io_error, json_error, now_secs, sanitize_component, SlackError, SlackProfileRuntime};
use crate::config::ChannelProfile;
use crate::queue::QueuePaths;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const INITIAL_HISTORY_WINDOW_SECS: i64 = 24 * 60 * 60;

fn default_oldest_timestamp() -> String {
    let oldest = now_secs().saturating_sub(INITIAL_HISTORY_WINDOW_SECS);
    format!("{oldest}.0")
}

fn resolve_oldest_cursor(existing: Option<&str>) -> String {
    existing
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(default_oldest_timestamp)
}

fn thread_cursor_key(conversation_id: &str, thread_ts: &str) -> String {
    format!("{conversation_id}:{thread_ts}")
}

fn parse_ts_from_message_id(message_id: &str) -> Option<String> {
    let (_, raw_ts) = message_id.rsplit_once('-')?;
    if raw_ts.trim().is_empty() {
        return None;
    }
    Some(raw_ts.replace('_', "."))
}

fn bootstrap_threads_from_artifacts(
    queue_paths: &QueuePaths,
    profile_id: &str,
    conversation_id: &str,
) -> BTreeSet<String> {
    let mut threads = BTreeSet::new();
    let artifacts_root = queue_paths.root.join("orchestrator/artifacts");
    let prefix = format!(
        "message-slack-{}-{}-",
        sanitize_component(profile_id),
        sanitize_component(conversation_id)
    );

    let Ok(entries) = std::fs::read_dir(&artifacts_root) else {
        return threads;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };

        let Some(conversation_ref) = value.get("conversationId").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some((channel_id, thread_ts)) = conversation_ref.split_once(':') else {
            continue;
        };
        if channel_id != conversation_id || thread_ts.trim().is_empty() {
            continue;
        }

        // Ignore top-level channel messages where thread id is the message ts.
        let is_thread_reply = value
            .get("messageId")
            .and_then(|v| v.as_str())
            .and_then(parse_ts_from_message_id)
            .map(|message_ts| message_ts != thread_ts)
            .unwrap_or(true);
        if !is_thread_reply {
            continue;
        }

        threads.insert(thread_ts.to_string());
    }

    threads
}

fn collect_thread_candidates(message: &SlackMessage, threads: &mut BTreeSet<String>) {
    if let Some(thread_ts) = message.thread_ts.as_deref() {
        if !thread_ts.trim().is_empty() {
            threads.insert(thread_ts.to_string());
            return;
        }
    }
    if message.reply_count.unwrap_or(0) > 0 && !message.ts.trim().is_empty() {
        threads.insert(message.ts.clone());
    }
}

trait SlackInboundApi {
    fn list_conversations(
        &self,
        include_im_conversations: bool,
    ) -> Result<Vec<ConversationSummary>, SlackError>;
    fn conversation_history(
        &self,
        conversation_id: &str,
        oldest: Option<&str>,
    ) -> Result<Vec<SlackMessage>, SlackError>;
    fn conversation_replies(
        &self,
        conversation_id: &str,
        thread_ts: &str,
        oldest: Option<&str>,
    ) -> Result<Vec<SlackMessage>, SlackError>;
}

impl SlackInboundApi for SlackApiClient {
    fn list_conversations(
        &self,
        include_im_conversations: bool,
    ) -> Result<Vec<ConversationSummary>, SlackError> {
        self.list_conversations(include_im_conversations)
    }

    fn conversation_history(
        &self,
        conversation_id: &str,
        oldest: Option<&str>,
    ) -> Result<Vec<SlackMessage>, SlackError> {
        self.conversation_history(conversation_id, oldest)
    }

    fn conversation_replies(
        &self,
        conversation_id: &str,
        thread_ts: &str,
        oldest: Option<&str>,
    ) -> Result<Vec<SlackMessage>, SlackError> {
        self.conversation_replies(conversation_id, thread_ts, oldest)
    }
}

pub fn should_accept_channel_message(
    _profile: &ChannelProfile,
    _allowlist: &BTreeSet<String>,
    _conversation_id: &str,
    _message_text: &str,
    _message_ts: &str,
    _thread_ts: Option<&str>,
) -> bool {
    true
}

pub(super) fn enqueue_incoming(
    queue_paths: &QueuePaths,
    profile_id: &str,
    profile: &ChannelProfile,
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
    if message_already_queued(queue_paths, &message_id)? {
        return Ok(false);
    }
    let path = queue_paths.incoming.join(format!("{message_id}.json"));
    let thread_ts = message.thread_ts.clone().unwrap_or_else(|| ts.clone());
    let message_text = message.text.clone().unwrap_or_default();
    let is_thread_reply = thread_ts != ts;

    let payload = crate::queue::IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some(profile_id.to_string()),
        sender: sender_id.clone(),
        sender_id,
        message: message_text.clone(),
        timestamp: now_secs(),
        message_id,
        conversation_id: Some(format!("{conversation_id}:{thread_ts}")),
        is_direct: conversation_id.trim_start().starts_with('D'),
        is_thread_reply,
        is_mentioned: message_is_mentioned(profile, &message_text),
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    let body = serde_json::to_vec_pretty(&payload).map_err(|e| json_error(&path, e))?;
    fs::write(&path, body).map_err(|e| io_error(&path, e))?;
    Ok(true)
}

fn message_is_mentioned(profile: &ChannelProfile, message_text: &str) -> bool {
    profile
        .mention_tokens()
        .iter()
        .any(|token| message_text.contains(token))
}

fn message_already_queued(queue_paths: &QueuePaths, message_id: &str) -> Result<bool, SlackError> {
    let incoming_path = queue_paths.incoming.join(format!("{message_id}.json"));
    if incoming_path.exists() {
        return Ok(true);
    }
    if queue_paths
        .processing
        .join(format!("{message_id}.json"))
        .exists()
    {
        return Ok(true);
    }
    if directory_has_outgoing_message(&queue_paths.outgoing, message_id)? {
        return Ok(true);
    }
    Ok(false)
}

fn directory_has_outgoing_message(directory: &Path, message_id: &str) -> Result<bool, SlackError> {
    let expected_prefix = format!("slack_{}_", sanitize_outgoing_component(message_id));
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(io_error(directory, err)),
    };
    for entry in entries {
        let entry = entry.map_err(|err| io_error(directory, err))?;
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if name.starts_with(&expected_prefix) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn sanitize_outgoing_component(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn process_inbound_for_profile(
    state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
) -> Result<usize, SlackError> {
    process_inbound_with_api(
        state_root,
        queue_paths,
        profile_id,
        &runtime.api,
        &runtime.profile,
        &runtime.allowlist,
        runtime.include_im_conversations,
    )
}

fn process_inbound_with_api<A: SlackInboundApi>(
    state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    api: &A,
    profile: &ChannelProfile,
    allowlist: &BTreeSet<String>,
    include_im_conversations: bool,
) -> Result<usize, SlackError> {
    let mut cursor_state = load_cursor_state(state_root, profile_id)?;
    let mut enqueued = 0usize;

    for conversation in api.list_conversations(include_im_conversations)? {
        let oldest = resolve_oldest_cursor(
            cursor_state
                .conversations
                .get(&conversation.id)
                .map(String::as_str),
        );
        let mut latest_ts = oldest.clone();
        let mut threads = BTreeSet::<String>::new();
        for key in cursor_state.threads.keys() {
            if let Some((channel_id, thread_ts)) = key.split_once(':') {
                if channel_id == conversation.id {
                    threads.insert(thread_ts.to_string());
                }
            }
        }
        if threads.is_empty() {
            threads.extend(bootstrap_threads_from_artifacts(
                queue_paths,
                profile_id,
                &conversation.id,
            ));
        }
        let messages = match api.conversation_history(&conversation.id, Some(oldest.as_str())) {
            Ok(messages) => messages,
            Err(SlackError::ApiResponse(message))
                if message.contains("conversations.history failed: not_in_channel") =>
            {
                continue;
            }
            Err(err) => return Err(err),
        };
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
                    profile,
                    allowlist,
                    &conversation.id,
                    message.text.as_deref().unwrap_or(""),
                    &message.ts,
                    message.thread_ts.as_deref(),
                )
            {
                continue;
            }

            if enqueue_incoming(queue_paths, profile_id, profile, &conversation.id, &message)? {
                enqueued += 1;
            }
            collect_thread_candidates(&message, &mut threads);
            if let Some(thread_ts) = message.thread_ts.as_deref() {
                if !thread_ts.trim().is_empty() {
                    let key = thread_cursor_key(&conversation.id, thread_ts);
                    let existing = cursor_state
                        .threads
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| oldest.clone());
                    if message.ts > existing {
                        cursor_state.threads.insert(key, message.ts.clone());
                    }
                }
            }

            if message.ts > latest_ts {
                latest_ts = message.ts;
            }
        }
        if threads.is_empty() {
            let bootstrap_oldest = default_oldest_timestamp();
            if let Ok(bootstrap_messages) =
                api.conversation_history(&conversation.id, Some(bootstrap_oldest.as_str()))
            {
                for message in bootstrap_messages {
                    collect_thread_candidates(&message, &mut threads);
                }
            }
        }

        for thread_ts in threads {
            let key = thread_cursor_key(&conversation.id, &thread_ts);
            let thread_oldest =
                resolve_oldest_cursor(cursor_state.threads.get(&key).map(String::as_str));
            let mut latest_thread_ts = thread_oldest.clone();
            let replies = match api.conversation_replies(
                &conversation.id,
                &thread_ts,
                Some(thread_oldest.as_str()),
            ) {
                Ok(messages) => messages,
                Err(SlackError::ApiResponse(message))
                    if message.contains("conversations.replies failed: not_in_channel") =>
                {
                    continue;
                }
                Err(err) => return Err(err),
            };

            for reply in replies {
                if reply.ts.trim().is_empty() || reply.ts == thread_ts {
                    continue;
                }
                if reply.user.is_none() {
                    continue;
                }
                if reply.bot_id.is_some() || reply.subtype.is_some() {
                    continue;
                }
                if !conversation.is_im
                    && !should_accept_channel_message(
                        profile,
                        allowlist,
                        &conversation.id,
                        reply.text.as_deref().unwrap_or(""),
                        &reply.ts,
                        reply.thread_ts.as_deref(),
                    )
                {
                    continue;
                }
                if enqueue_incoming(queue_paths, profile_id, profile, &conversation.id, &reply)? {
                    enqueued += 1;
                }
                if reply.ts > latest_thread_ts {
                    latest_thread_ts = reply.ts.clone();
                }
                if reply.ts > latest_ts {
                    latest_ts = reply.ts;
                }
            }

            if latest_thread_ts > thread_oldest {
                cursor_state.threads.insert(key, latest_thread_ts);
            }
        }

        cursor_state
            .conversations
            .insert(conversation.id.clone(), latest_ts);
    }

    save_cursor_state(state_root, profile_id, &cursor_state)?;
    Ok(enqueued)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelKind, ChannelProfile, ThreadResponseMode};
    use crate::queue::IncomingMessage;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    #[derive(Debug, Default)]
    struct MockInboundApi {
        conversations: Vec<ConversationSummary>,
        history_by_conversation: BTreeMap<String, Vec<SlackMessage>>,
        bootstrap_history_by_conversation: BTreeMap<String, Vec<SlackMessage>>,
        replies_by_thread: BTreeMap<String, Vec<SlackMessage>>,
    }

    impl MockInboundApi {
        fn thread_key(conversation_id: &str, thread_ts: &str) -> String {
            format!("{conversation_id}:{thread_ts}")
        }
    }

    impl SlackInboundApi for MockInboundApi {
        fn list_conversations(
            &self,
            _include_im_conversations: bool,
        ) -> Result<Vec<ConversationSummary>, SlackError> {
            Ok(self.conversations.clone())
        }

        fn conversation_history(
            &self,
            conversation_id: &str,
            oldest: Option<&str>,
        ) -> Result<Vec<SlackMessage>, SlackError> {
            if oldest == Some("500.0") {
                return Ok(self
                    .history_by_conversation
                    .get(conversation_id)
                    .cloned()
                    .unwrap_or_default());
            }
            if let Some(messages) = self.bootstrap_history_by_conversation.get(conversation_id) {
                return Ok(messages.clone());
            }
            Ok(self
                .history_by_conversation
                .get(conversation_id)
                .cloned()
                .unwrap_or_default())
        }

        fn conversation_replies(
            &self,
            conversation_id: &str,
            thread_ts: &str,
            _oldest: Option<&str>,
        ) -> Result<Vec<SlackMessage>, SlackError> {
            Ok(self
                .replies_by_thread
                .get(&Self::thread_key(conversation_id, thread_ts))
                .cloned()
                .unwrap_or_default())
        }
    }

    fn profile(slack_app_user_id: Option<&str>) -> ChannelProfile {
        ChannelProfile {
            channel: ChannelKind::Slack,
            orchestrator_id: "orch.main".to_string(),
            identity: Default::default(),
            slack_app_user_id: slack_app_user_id.map(|v| v.to_string()),
            require_mention_in_channels: Some(true),
            thread_response_mode: ThreadResponseMode::AlwaysReply,
        }
    }

    fn setup_queue_paths(root: &Path) -> QueuePaths {
        let queue_paths = QueuePaths::from_state_root(root);
        std::fs::create_dir_all(&queue_paths.incoming).expect("incoming");
        std::fs::create_dir_all(&queue_paths.processing).expect("processing");
        std::fs::create_dir_all(&queue_paths.outgoing).expect("outgoing");
        queue_paths
    }

    fn inbox_file_count(queue_paths: &QueuePaths) -> usize {
        std::fs::read_dir(&queue_paths.incoming)
            .expect("read incoming")
            .count()
    }

    fn slack_message(ts: &str, thread_ts: Option<&str>, text: &str) -> SlackMessage {
        SlackMessage {
            ts: ts.to_string(),
            thread_ts: thread_ts.map(|v| v.to_string()),
            text: Some(text.to_string()),
            user: Some("U123".to_string()),
            subtype: None,
            bot_id: None,
            reply_count: None,
        }
    }

    fn slack_message_id(profile_id: &str, conversation_id: &str, ts: &str) -> String {
        format!(
            "slack-{}-{}-{}",
            super::sanitize_component(profile_id),
            super::sanitize_component(conversation_id),
            super::sanitize_component(ts)
        )
    }

    #[test]
    fn ingests_channel_thread_replies_when_root_is_older_than_channel_cursor() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join("state");
        let queue_root = temp.path().join("runtime");
        let queue_paths = setup_queue_paths(&queue_root);

        let mut cursor_state = super::load_cursor_state(&state_root, "profile.main").expect("load");
        cursor_state
            .conversations
            .insert("C001".to_string(), "200.0".to_string());
        cursor_state
            .threads
            .insert("C001:100.0".to_string(), "200.0".to_string());
        super::save_cursor_state(&state_root, "profile.main", &cursor_state).expect("save");

        let mut api = MockInboundApi::default();
        api.conversations.push(ConversationSummary {
            id: "C001".to_string(),
            is_im: false,
        });
        api.replies_by_thread.insert(
            MockInboundApi::thread_key("C001", "100.0"),
            vec![slack_message("250.0", Some("100.0"), "reply")],
        );

        let enqueued = process_inbound_with_api(
            &state_root,
            &queue_paths,
            "profile.main",
            &api,
            &profile(Some("UAPP")),
            &BTreeSet::new(),
            true,
        )
        .expect("process");
        assert_eq!(enqueued, 1);
        assert_eq!(inbox_file_count(&queue_paths), 1);
    }

    #[test]
    fn non_thread_channel_message_is_still_ingested_for_opportunistic_policy() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join("state");
        let queue_root = temp.path().join("runtime");
        let queue_paths = setup_queue_paths(&queue_root);

        let mut api = MockInboundApi::default();
        api.conversations.push(ConversationSummary {
            id: "C001".to_string(),
            is_im: false,
        });
        api.history_by_conversation.insert(
            "C001".to_string(),
            vec![slack_message("300.0", None, "no mention")],
        );

        let enqueued = process_inbound_with_api(
            &state_root,
            &queue_paths,
            "profile.main",
            &api,
            &profile(Some("UAPP")),
            &BTreeSet::new(),
            true,
        )
        .expect("process");
        assert_eq!(enqueued, 1);
        assert_eq!(inbox_file_count(&queue_paths), 1);
    }

    #[test]
    fn dedupes_when_the_same_thread_message_appears_in_history_and_replies_paths() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join("state");
        let queue_root = temp.path().join("runtime");
        let queue_paths = setup_queue_paths(&queue_root);

        let mut api = MockInboundApi::default();
        api.conversations.push(ConversationSummary {
            id: "C001".to_string(),
            is_im: false,
        });
        api.history_by_conversation.insert(
            "C001".to_string(),
            vec![slack_message("300.0", Some("100.0"), "same message")],
        );
        api.replies_by_thread.insert(
            MockInboundApi::thread_key("C001", "100.0"),
            vec![slack_message("300.0", Some("100.0"), "same message")],
        );

        let enqueued = process_inbound_with_api(
            &state_root,
            &queue_paths,
            "profile.main",
            &api,
            &profile(Some("UAPP")),
            &BTreeSet::new(),
            true,
        )
        .expect("process");
        assert_eq!(enqueued, 1);
        assert_eq!(inbox_file_count(&queue_paths), 1);
    }

    #[test]
    fn dedupes_when_message_is_already_in_processing_queue() {
        let temp = tempdir().expect("tempdir");
        let queue_root = temp.path().join("runtime");
        let queue_paths = setup_queue_paths(&queue_root);
        let profile_id = "profile.main";
        let conversation_id = "C001";
        let ts = "300.0";
        let message_id = slack_message_id(profile_id, conversation_id, ts);

        let processing_payload = IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some(profile_id.to_string()),
            sender: "U123".to_string(),
            sender_id: "U123".to_string(),
            message: "already processing".to_string(),
            timestamp: 1,
            message_id: message_id.clone(),
            conversation_id: Some("C001:100.0".to_string()),
            is_direct: false,
            is_thread_reply: false,
            is_mentioned: false,
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        };
        std::fs::write(
            queue_paths.processing.join(format!("{message_id}.json")),
            serde_json::to_vec_pretty(&processing_payload).expect("serialize processing"),
        )
        .expect("write processing");

        let enqueued = enqueue_incoming(
            &queue_paths,
            profile_id,
            &profile(Some("UAPP")),
            conversation_id,
            &slack_message(ts, Some("100.0"), "same"),
        )
        .expect("enqueue");
        assert!(!enqueued);
        assert_eq!(inbox_file_count(&queue_paths), 0);
    }

    #[test]
    fn dedupes_when_message_is_already_in_outgoing_queue() {
        let temp = tempdir().expect("tempdir");
        let queue_root = temp.path().join("runtime");
        let queue_paths = setup_queue_paths(&queue_root);
        let profile_id = "profile.main";
        let conversation_id = "C001";
        let ts = "300.0";
        let message_id = slack_message_id(profile_id, conversation_id, ts);
        let outgoing_name = format!("slack_{}_1.json", message_id);
        std::fs::write(queue_paths.outgoing.join(outgoing_name), "{}").expect("write outgoing");

        let enqueued = enqueue_incoming(
            &queue_paths,
            profile_id,
            &profile(Some("UAPP")),
            conversation_id,
            &slack_message(ts, Some("100.0"), "same"),
        )
        .expect("enqueue");
        assert!(!enqueued);
        assert_eq!(inbox_file_count(&queue_paths), 0);
    }

    #[test]
    fn enqueue_ignores_unreadable_unrelated_outgoing_json() {
        let temp = tempdir().expect("tempdir");
        let queue_root = temp.path().join("runtime");
        let queue_paths = setup_queue_paths(&queue_root);

        std::fs::write(
            queue_paths.outgoing.join("unrelated.json"),
            vec![0xff, 0xfe, 0xfd],
        )
        .expect("write invalid utf8");

        let enqueued = enqueue_incoming(
            &queue_paths,
            "profile.main",
            &profile(Some("UAPP")),
            "C001",
            &slack_message("301.0", Some("100.0"), "new"),
        )
        .expect("enqueue");
        assert!(enqueued);
        assert_eq!(inbox_file_count(&queue_paths), 1);
    }

    #[test]
    fn bootstraps_known_threads_from_local_artifacts_when_thread_cursor_is_missing() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join("state");
        let queue_root = temp.path().join("runtime");
        let queue_paths = setup_queue_paths(&queue_root);
        let artifacts = queue_root.join("orchestrator/artifacts");
        std::fs::create_dir_all(&artifacts).expect("artifacts dir");
        std::fs::write(
            artifacts.join("message-slack-profile_main-C001-200_0.json"),
            r#"{
  "messageId": "slack-profile_main-C001-200_0",
  "conversationId": "C001:100.0"
}"#,
        )
        .expect("write artifact");

        let mut cursor_state = super::load_cursor_state(&state_root, "profile.main").expect("load");
        cursor_state
            .conversations
            .insert("C001".to_string(), "500.0".to_string());
        super::save_cursor_state(&state_root, "profile.main", &cursor_state).expect("save");

        let mut api = MockInboundApi::default();
        api.conversations.push(ConversationSummary {
            id: "C001".to_string(),
            is_im: false,
        });
        api.replies_by_thread.insert(
            MockInboundApi::thread_key("C001", "100.0"),
            vec![slack_message(
                "9999999999.0",
                Some("100.0"),
                "bootstrapped reply",
            )],
        );

        let enqueued = process_inbound_with_api(
            &state_root,
            &queue_paths,
            "profile.main",
            &api,
            &profile(Some("UAPP")),
            &BTreeSet::new(),
            true,
        )
        .expect("process");
        assert_eq!(enqueued, 1);
        assert_eq!(inbox_file_count(&queue_paths), 1);
    }

    #[test]
    fn bootstraps_thread_discovery_from_recent_history_when_no_thread_cursor_exists() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join("state");
        let queue_root = temp.path().join("runtime");
        let queue_paths = setup_queue_paths(&queue_root);

        let mut cursor_state = super::load_cursor_state(&state_root, "profile.main").expect("load");
        cursor_state
            .conversations
            .insert("C001".to_string(), "500.0".to_string());
        super::save_cursor_state(&state_root, "profile.main", &cursor_state).expect("save");

        let mut api = MockInboundApi::default();
        api.conversations.push(ConversationSummary {
            id: "C001".to_string(),
            is_im: false,
        });
        api.bootstrap_history_by_conversation.insert(
            "C001".to_string(),
            vec![SlackMessage {
                ts: "100.0".to_string(),
                thread_ts: None,
                text: Some("root".to_string()),
                user: Some("U123".to_string()),
                subtype: None,
                bot_id: None,
                reply_count: Some(1),
            }],
        );
        api.replies_by_thread.insert(
            MockInboundApi::thread_key("C001", "100.0"),
            vec![slack_message(
                "9999999999.0",
                Some("100.0"),
                "reply from discovered root",
            )],
        );

        let enqueued = process_inbound_with_api(
            &state_root,
            &queue_paths,
            "profile.main",
            &api,
            &profile(Some("UAPP")),
            &BTreeSet::new(),
            true,
        )
        .expect("process");
        assert_eq!(enqueued, 1);
        assert_eq!(inbox_file_count(&queue_paths), 1);
    }
}
