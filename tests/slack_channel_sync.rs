use direclaw::channels::slack::{sync_once, SlackError};
use direclaw::config::{
    AuthSyncConfig, ChannelConfig, ChannelKind, ChannelProfile, Monitoring, Settings,
    SettingsOrchestrator,
};
use direclaw::queue::{OutgoingMessage, QueuePaths};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
struct RecordedRequest {
    path: String,
    auth_header: String,
    body: String,
}

struct MockSlackServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockSlackServer {
    fn start<F>(expected_requests: usize, responder: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_thread = Arc::clone(&requests);
        let responder = Arc::new(responder);

        let handle = thread::spawn(move || {
            for _ in 0..expected_requests {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

                let mut request_line = String::new();
                reader
                    .read_line(&mut request_line)
                    .expect("read request line");
                let mut path = "/".to_string();
                if let Some(raw_path) = request_line.split_whitespace().nth(1) {
                    path = raw_path.to_string();
                }

                let mut auth_header = String::new();
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("read header");
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if lower.starts_with("authorization:") {
                        auth_header = line
                            .split_once(':')
                            .map(|(_, v)| v.trim().to_string())
                            .unwrap_or_default();
                    }
                    if lower.starts_with("content-length:") {
                        content_length = line
                            .split_once(':')
                            .map(|(_, v)| v.trim().parse::<usize>().unwrap_or(0))
                            .unwrap_or(0);
                    }
                }

                let mut body = vec![0_u8; content_length];
                if content_length > 0 {
                    reader.read_exact(&mut body).expect("read body");
                }
                let body = String::from_utf8_lossy(&body).to_string();

                requests_for_thread
                    .lock()
                    .expect("lock requests")
                    .push(RecordedRequest {
                        path: path.clone(),
                        auth_header,
                        body,
                    });

                let response_body = responder(&path);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        Self {
            base_url: format!("http://{}", addr),
            requests,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> Vec<RecordedRequest> {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join mock server");
        }
        self.requests.lock().expect("lock requests").clone()
    }
}

fn sample_settings(
    workspaces_path: &Path,
    require_mention: bool,
    allowlisted_channels: Vec<String>,
) -> Settings {
    let mut orchestrators = BTreeMap::new();
    orchestrators.insert(
        "main".to_string(),
        SettingsOrchestrator {
            private_workspace: None,
            shared_access: Vec::new(),
        },
    );

    let mut channel_profiles = BTreeMap::new();
    channel_profiles.insert(
        "slack_main".to_string(),
        ChannelProfile {
            channel: ChannelKind::Slack,
            orchestrator_id: "main".to_string(),
            slack_app_user_id: Some("UAPP".to_string()),
            require_mention_in_channels: Some(require_mention),
        },
    );

    let mut channels = BTreeMap::new();
    channels.insert(
        "slack".to_string(),
        ChannelConfig {
            enabled: true,
            allowlisted_channels,
        },
    );

    Settings {
        workspaces_path: workspaces_path.to_path_buf(),
        shared_workspaces: BTreeMap::new(),
        orchestrators,
        channel_profiles,
        monitoring: Monitoring::default(),
        channels,
        auth_sync: AuthSyncConfig::default(),
    }
}

fn queue_for_profile(settings: &Settings, profile_id: &str) -> QueuePaths {
    let runtime_root = settings
        .resolve_channel_profile_runtime_root(profile_id)
        .expect("runtime root");
    QueuePaths::from_state_root(&runtime_root)
}

fn set_env(base_url: &str) {
    std::env::set_var("DIRECLAW_SLACK_API_BASE", format!("{base_url}/api"));
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");
    std::env::remove_var("SLACK_CHANNEL_ALLOWLIST");
    std::env::remove_var("SLACK_BOT_TOKEN_SLACK_MAIN");
    std::env::remove_var("SLACK_APP_TOKEN_SLACK_MAIN");
    std::env::remove_var("SLACK_BOT_TOKEN_SLACK_ALT");
    std::env::remove_var("SLACK_APP_TOKEN_SLACK_ALT");
}

#[test]
fn sync_requires_slack_env_tokens() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("DIRECLAW_SLACK_API_BASE");
    std::env::remove_var("SLACK_CHANNEL_ALLOWLIST");

    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");
    fs::create_dir_all(&state_root).expect("state root");
    let settings = sample_settings(temp.path(), true, Vec::new());

    let err = sync_once(&state_root, &settings).expect_err("missing env should fail");
    assert!(matches!(
        err,
        SlackError::MissingProfileScopedEnvVar { ref profile_id, ref key }
            if profile_id == "slack_main" && key == "SLACK_BOT_TOKEN_SLACK_MAIN"
    ));
}

#[test]
fn sync_queues_inbound_and_sends_outbound() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let server = MockSlackServer::start(5, |path| {
        if path.starts_with("/api/auth.test") {
            return r#"{"ok":true}"#.to_string();
        }
        if path.starts_with("/api/apps.connections.open") {
            return r#"{"ok":true,"url":"wss://example"}"#.to_string();
        }
        if path.starts_with("/api/conversations.list") {
            return r#"{"ok":true,"conversations":[{"id":"D111","is_im":true}],"response_metadata":{"next_cursor":""}}"#.to_string();
        }
        if path.starts_with("/api/conversations.history") {
            return r#"{"ok":true,"messages":[{"ts":"1700000000.1","text":"hello from slack","user":"U123"}]}"#
                .to_string();
        }
        if path.starts_with("/api/chat.postMessage") {
            return r#"{"ok":true,"ts":"1700000000.2"}"#.to_string();
        }
        r#"{"ok":false,"error":"unexpected_path"}"#.to_string()
    });
    set_env(&server.base_url);

    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");
    let settings = sample_settings(temp.path(), true, Vec::new());
    let queue = queue_for_profile(&settings, "slack_main");
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
    fs::create_dir_all(&queue.outgoing).expect("outgoing dir");

    let outbound_path = queue.outgoing.join("slack_msg_1_1.json");
    let outbound = OutgoingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("slack_main".to_string()),
        sender: "assistant".to_string(),
        message: "outbound reply".to_string(),
        original_message: "original".to_string(),
        timestamp: 1,
        message_id: "msg_1".to_string(),
        agent: "agent-a".to_string(),
        conversation_id: Some("D111:1700000000.1".to_string()),
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    fs::write(
        &outbound_path,
        serde_json::to_string_pretty(&outbound).expect("encode outbound"),
    )
    .expect("write outbound");

    let report = sync_once(&state_root, &settings).expect("sync succeeds");
    assert_eq!(report.profiles_processed, 1);
    assert_eq!(report.inbound_enqueued, 1);
    assert_eq!(report.outbound_messages_sent, 1);
    assert!(!outbound_path.exists(), "outgoing file should be consumed");

    let incoming_files = fs::read_dir(&queue.incoming)
        .expect("incoming list")
        .map(|entry| entry.expect("entry").path())
        .collect::<Vec<_>>();
    assert_eq!(incoming_files.len(), 1);
    let raw = fs::read_to_string(&incoming_files[0]).expect("read incoming");
    assert!(raw.contains("\"channelProfileId\": \"slack_main\""));
    assert!(raw.contains("\"message\": \"hello from slack\""));

    let requests = server.finish();
    let post = requests
        .iter()
        .find(|request| request.path.starts_with("/api/chat.postMessage"))
        .expect("post request");
    assert_eq!(post.auth_header, "Bearer xoxb-test");
    assert!(post.body.contains("\"channel\":\"D111\""));
    assert!(post.body.contains("\"thread_ts\":\"1700000000.1\""));
}

#[test]
fn sync_respects_require_mention_for_channels() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let server = MockSlackServer::start(4, |path| {
        if path.starts_with("/api/auth.test") {
            return r#"{"ok":true}"#.to_string();
        }
        if path.starts_with("/api/apps.connections.open") {
            return r#"{"ok":true,"url":"wss://example"}"#.to_string();
        }
        if path.starts_with("/api/conversations.list") {
            return r#"{"ok":true,"conversations":[{"id":"C222","is_im":false}],"response_metadata":{"next_cursor":""}}"#.to_string();
        }
        if path.starts_with("/api/conversations.history") {
            return r#"{"ok":true,"messages":[{"ts":"1700000000.1","text":"plain channel note","user":"U123"}]}"#
                .to_string();
        }
        r#"{"ok":false,"error":"unexpected_path"}"#.to_string()
    });
    set_env(&server.base_url);

    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");
    let settings = sample_settings(temp.path(), true, Vec::new());
    let queue = queue_for_profile(&settings, "slack_main");
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
    fs::create_dir_all(&queue.outgoing).expect("outgoing dir");
    let report = sync_once(&state_root, &settings).expect("sync succeeds");
    assert_eq!(report.inbound_enqueued, 0);

    let incoming_count = fs::read_dir(&queue.incoming)
        .expect("incoming list")
        .count();
    assert_eq!(incoming_count, 0);
    let _ = server.finish();
}

#[test]
fn sync_requires_allowlist_thread_or_mention_even_when_mentions_not_required() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let server = MockSlackServer::start(4, |path| {
        if path.starts_with("/api/auth.test") {
            return r#"{"ok":true}"#.to_string();
        }
        if path.starts_with("/api/apps.connections.open") {
            return r#"{"ok":true,"url":"wss://example"}"#.to_string();
        }
        if path.starts_with("/api/conversations.list") {
            return r#"{"ok":true,"conversations":[{"id":"C222","is_im":false}],"response_metadata":{"next_cursor":""}}"#.to_string();
        }
        if path.starts_with("/api/conversations.history") {
            return r#"{"ok":true,"messages":[{"ts":"1700000000.1","text":"plain channel note","user":"U123"}]}"#
                .to_string();
        }
        r#"{"ok":false,"error":"unexpected_path"}"#.to_string()
    });
    set_env(&server.base_url);

    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");
    let settings = sample_settings(temp.path(), false, Vec::new());
    let queue = queue_for_profile(&settings, "slack_main");
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
    fs::create_dir_all(&queue.outgoing).expect("outgoing dir");
    let report = sync_once(&state_root, &settings).expect("sync succeeds");
    assert_eq!(report.inbound_enqueued, 0);
    assert_eq!(
        fs::read_dir(&queue.incoming)
            .expect("incoming list")
            .count(),
        0
    );
    let _ = server.finish();
}

#[test]
fn sync_uses_configured_allowlisted_channels() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let server = MockSlackServer::start(4, |path| {
        if path.starts_with("/api/auth.test") {
            return r#"{"ok":true}"#.to_string();
        }
        if path.starts_with("/api/apps.connections.open") {
            return r#"{"ok":true,"url":"wss://example"}"#.to_string();
        }
        if path.starts_with("/api/conversations.list") {
            return r#"{"ok":true,"conversations":[{"id":"C222","is_im":false}],"response_metadata":{"next_cursor":""}}"#.to_string();
        }
        if path.starts_with("/api/conversations.history") {
            return r#"{"ok":true,"messages":[{"ts":"1700000000.1","text":"plain channel note","user":"U123"}]}"#
                .to_string();
        }
        r#"{"ok":false,"error":"unexpected_path"}"#.to_string()
    });
    set_env(&server.base_url);

    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");
    let settings = sample_settings(temp.path(), true, vec!["C222".to_string()]);
    let queue = queue_for_profile(&settings, "slack_main");
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
    fs::create_dir_all(&queue.outgoing).expect("outgoing dir");
    let report = sync_once(&state_root, &settings).expect("sync succeeds");
    assert_eq!(report.inbound_enqueued, 1);
    assert_eq!(
        fs::read_dir(&queue.incoming)
            .expect("incoming list")
            .count(),
        1
    );
    let _ = server.finish();
}

#[test]
fn sync_pages_conversation_history_before_advancing_cursor() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let server = MockSlackServer::start(5, |path| {
        if path.starts_with("/api/auth.test") {
            return r#"{"ok":true}"#.to_string();
        }
        if path.starts_with("/api/apps.connections.open") {
            return r#"{"ok":true,"url":"wss://example"}"#.to_string();
        }
        if path.starts_with("/api/conversations.list") {
            return r#"{"ok":true,"conversations":[{"id":"D111","is_im":true}],"response_metadata":{"next_cursor":""}}"#.to_string();
        }
        if path.starts_with("/api/conversations.history") && !path.contains("cursor=") {
            return r#"{"ok":true,"messages":[{"ts":"1700000000.1","text":"first page","user":"U123"}],"response_metadata":{"next_cursor":"cursor-2"}}"#.to_string();
        }
        if path.starts_with("/api/conversations.history") && path.contains("cursor=cursor-2") {
            return r#"{"ok":true,"messages":[{"ts":"1700000000.2","text":"second page","user":"U123"}],"response_metadata":{"next_cursor":""}}"#.to_string();
        }
        r#"{"ok":false,"error":"unexpected_path"}"#.to_string()
    });
    set_env(&server.base_url);

    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");
    let settings = sample_settings(temp.path(), true, Vec::new());
    let queue = queue_for_profile(&settings, "slack_main");
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
    fs::create_dir_all(&queue.outgoing).expect("outgoing dir");
    let report = sync_once(&state_root, &settings).expect("sync succeeds");
    assert_eq!(report.inbound_enqueued, 2);
    assert_eq!(
        fs::read_dir(&queue.incoming)
            .expect("incoming list")
            .count(),
        2
    );
    let _ = server.finish();
}

#[test]
fn sync_requires_profile_scoped_tokens_for_multiple_profiles() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-global");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-global");
    std::env::remove_var("SLACK_BOT_TOKEN_SLACK_MAIN");
    std::env::remove_var("SLACK_APP_TOKEN_SLACK_MAIN");
    std::env::remove_var("SLACK_BOT_TOKEN_SLACK_ALT");
    std::env::remove_var("SLACK_APP_TOKEN_SLACK_ALT");
    std::env::remove_var("DIRECLAW_SLACK_API_BASE");
    std::env::remove_var("SLACK_CHANNEL_ALLOWLIST");

    let temp = tempdir().expect("tempdir");
    let mut settings = sample_settings(temp.path(), true, Vec::new());
    settings.channel_profiles.insert(
        "slack_alt".to_string(),
        ChannelProfile {
            channel: ChannelKind::Slack,
            orchestrator_id: "main".to_string(),
            slack_app_user_id: Some("UAPPALT".to_string()),
            require_mention_in_channels: Some(true),
        },
    );

    let state_root = temp.path().join(".direclaw");
    fs::create_dir_all(&state_root).expect("state root");
    let err = sync_once(&state_root, &settings).expect_err("missing scoped env should fail");
    assert!(matches!(err, SlackError::MissingProfileScopedEnvVar { .. }));
}

#[test]
fn sync_avoids_duplicate_ingestion_on_repeated_polling() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let server = MockSlackServer::start(8, |path| {
        if path.starts_with("/api/auth.test") {
            return r#"{"ok":true}"#.to_string();
        }
        if path.starts_with("/api/apps.connections.open") {
            return r#"{"ok":true,"url":"wss://example"}"#.to_string();
        }
        if path.starts_with("/api/conversations.list") {
            return r#"{"ok":true,"conversations":[{"id":"D111","is_im":true}],"response_metadata":{"next_cursor":""}}"#.to_string();
        }
        if path.starts_with("/api/conversations.history") {
            return r#"{"ok":true,"messages":[{"ts":"1700000000.1","thread_ts":"1700000000.1","text":"hello once","user":"U123"}]}"#.to_string();
        }
        r#"{"ok":false,"error":"unexpected_path"}"#.to_string()
    });
    set_env(&server.base_url);

    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");
    let settings = sample_settings(temp.path(), true, Vec::new());
    let queue = queue_for_profile(&settings, "slack_main");
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
    fs::create_dir_all(&queue.outgoing).expect("outgoing dir");
    let first = sync_once(&state_root, &settings).expect("first sync");
    let second = sync_once(&state_root, &settings).expect("second sync");

    assert_eq!(first.inbound_enqueued, 1);
    assert_eq!(second.inbound_enqueued, 0);

    let incoming_files = fs::read_dir(&queue.incoming)
        .expect("incoming list")
        .map(|entry| entry.expect("entry").path())
        .collect::<Vec<_>>();
    assert_eq!(incoming_files.len(), 1);

    let raw = fs::read_to_string(&incoming_files[0]).expect("read incoming");
    assert!(raw.contains("\"channelProfileId\": \"slack_main\""));
    assert!(raw.contains("\"conversationId\": \"D111:1700000000.1\""));
    let _ = server.finish();
}

#[test]
fn sync_chunks_outbound_and_removes_queue_file_on_success() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let server = MockSlackServer::start(6, |path| {
        if path.starts_with("/api/auth.test") {
            return r#"{"ok":true}"#.to_string();
        }
        if path.starts_with("/api/apps.connections.open") {
            return r#"{"ok":true,"url":"wss://example"}"#.to_string();
        }
        if path.starts_with("/api/conversations.list") {
            return r#"{"ok":true,"conversations":[{"id":"D111","is_im":true}],"response_metadata":{"next_cursor":""}}"#.to_string();
        }
        if path.starts_with("/api/conversations.history") {
            return r#"{"ok":true,"messages":[],"response_metadata":{"next_cursor":""}}"#
                .to_string();
        }
        if path.starts_with("/api/chat.postMessage") {
            return r#"{"ok":true,"ts":"1700000000.2"}"#.to_string();
        }
        r#"{"ok":false,"error":"unexpected_path"}"#.to_string()
    });
    set_env(&server.base_url);

    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");
    let settings = sample_settings(temp.path(), true, Vec::new());
    let queue = queue_for_profile(&settings, "slack_main");
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
    fs::create_dir_all(&queue.outgoing).expect("outgoing dir");

    let outbound_path = queue.outgoing.join("slack_msg_chunked.json");
    let outbound = OutgoingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("slack_main".to_string()),
        sender: "assistant".to_string(),
        message: "x".repeat(3501),
        original_message: "original".to_string(),
        timestamp: 1,
        message_id: "msg_chunked".to_string(),
        agent: "agent-a".to_string(),
        conversation_id: Some("D111:1700000000.1".to_string()),
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    fs::write(
        &outbound_path,
        serde_json::to_string_pretty(&outbound).expect("encode outbound"),
    )
    .expect("write outbound");

    let report = sync_once(&state_root, &settings).expect("sync succeeds");
    assert_eq!(report.outbound_messages_sent, 1);
    assert!(!outbound_path.exists(), "outgoing file should be consumed");

    let requests = server.finish();
    let posts = requests
        .iter()
        .filter(|request| request.path.starts_with("/api/chat.postMessage"))
        .collect::<Vec<_>>();
    assert_eq!(posts.len(), 2);
    assert!(posts
        .iter()
        .all(|request| request.body.contains("\"thread_ts\":\"1700000000.1\"")));
}

#[test]
fn sync_preserves_outbound_file_and_returns_context_on_api_failure() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let server = MockSlackServer::start(5, |path| {
        if path.starts_with("/api/auth.test") {
            return r#"{"ok":true}"#.to_string();
        }
        if path.starts_with("/api/apps.connections.open") {
            return r#"{"ok":true,"url":"wss://example"}"#.to_string();
        }
        if path.starts_with("/api/conversations.list") {
            return r#"{"ok":true,"conversations":[{"id":"D111","is_im":true}],"response_metadata":{"next_cursor":""}}"#.to_string();
        }
        if path.starts_with("/api/conversations.history") {
            return r#"{"ok":true,"messages":[],"response_metadata":{"next_cursor":""}}"#
                .to_string();
        }
        if path.starts_with("/api/chat.postMessage") {
            return r#"{"ok":false,"error":"ratelimited"}"#.to_string();
        }
        r#"{"ok":false,"error":"unexpected_path"}"#.to_string()
    });
    set_env(&server.base_url);

    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");
    let settings = sample_settings(temp.path(), true, Vec::new());
    let queue = queue_for_profile(&settings, "slack_main");
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
    fs::create_dir_all(&queue.outgoing).expect("outgoing dir");

    let outbound_path = queue.outgoing.join("slack_msg_fail.json");
    let outbound = OutgoingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("slack_main".to_string()),
        sender: "assistant".to_string(),
        message: "outbound reply".to_string(),
        original_message: "original".to_string(),
        timestamp: 1,
        message_id: "msg_fail".to_string(),
        agent: "agent-a".to_string(),
        conversation_id: Some("D111:1700000000.1".to_string()),
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    fs::write(
        &outbound_path,
        serde_json::to_string_pretty(&outbound).expect("encode outbound"),
    )
    .expect("write outbound");

    let err = sync_once(&state_root, &settings).expect_err("sync should fail");
    let error_text = err.to_string();
    assert!(error_text.contains("msg_fail"));
    assert!(error_text.contains("slack_main"));
    assert!(error_text.contains("ratelimited"));
    assert!(
        outbound_path.exists(),
        "failed outbound file should be preserved for retry"
    );
    let _ = server.finish();
}
