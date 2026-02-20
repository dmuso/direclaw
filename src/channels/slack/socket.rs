use super::api::SlackMessage;
use super::ingest::{enqueue_incoming, should_accept_channel_message};
use super::{SlackError, SlackProfileRuntime};
use crate::config::ChannelProfile;
use crate::queue::QueuePaths;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const SOCKET_IDLE_SLEEP: Duration = Duration::from_millis(40);
const SOCKET_BUFFER_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryClass {
    Retryable,
    NonRetryable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SocketHealth {
    pub connected: bool,
    pub last_event_ts: Option<String>,
    pub last_reconnect: Option<i64>,
    pub last_error: Option<String>,
}

fn socket_status_path(state_root: &Path, profile_id: &str) -> PathBuf {
    state_root
        .join("channels/slack/socket")
        .join(format!("{profile_id}.health.json"))
}

fn reconnect_request_path(state_root: &Path) -> PathBuf {
    state_root.join("channels/slack/socket/reconnect.request")
}

fn load_health(state_root: &Path, profile_id: &str) -> SocketHealth {
    let path = socket_status_path(state_root, profile_id);
    let Ok(raw) = fs::read_to_string(&path) else {
        return SocketHealth::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_health(state_root: &Path, profile_id: &str, health: &SocketHealth) {
    let path = socket_status_path(state_root, profile_id);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_vec_pretty(health) {
        let _ = fs::write(&path, body);
    }
}

pub(super) fn read_profile_health(state_root: &Path, profile_id: &str) -> SocketHealth {
    load_health(state_root, profile_id)
}

pub(super) fn request_reconnect(state_root: &Path) -> Result<(), SlackError> {
    let path = reconnect_request_path(state_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| SlackError::Io {
            path: parent.display().to_string(),
            source: err,
        })?;
    }
    fs::write(&path, b"1").map_err(|err| SlackError::Io {
        path: path.display().to_string(),
        source: err,
    })?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct SocketEnvelope {
    #[serde(default)]
    envelope_id: Option<String>,
    #[serde(default)]
    payload: Option<SocketPayload>,
}

#[derive(Debug, Deserialize)]
struct SocketPayload {
    #[serde(default)]
    event: Option<SocketEvent>,
}

#[derive(Debug, Deserialize)]
struct SocketEvent {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    channel_type: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    bot_id: Option<String>,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    ts: String,
    #[serde(default)]
    thread_ts: Option<String>,
}

pub(super) fn process_socket_inbound_for_profile(
    state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
    reconnect_backoff_ms: u64,
    idle_timeout_ms: u64,
) -> Result<usize, SlackError> {
    let deadline = Instant::now() + Duration::from_millis(idle_timeout_ms.max(1));
    run_socket_loop(
        state_root,
        queue_paths,
        profile_id,
        runtime,
        reconnect_backoff_ms,
        Some(deadline),
        None,
    )
}

pub(super) fn run_socket_inbound_for_profile_until_stop(
    state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
    reconnect_backoff_ms: u64,
    stop: &AtomicBool,
) -> Result<usize, SlackError> {
    run_socket_loop(
        state_root,
        queue_paths,
        profile_id,
        runtime,
        reconnect_backoff_ms,
        None,
        Some(stop),
    )
}

fn run_socket_loop(
    state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
    reconnect_backoff_ms: u64,
    deadline: Option<Instant>,
    stop: Option<&AtomicBool>,
) -> Result<usize, SlackError> {
    let mut health = load_health(state_root, profile_id);
    let reconnect_backoff = Duration::from_millis(reconnect_backoff_ms.max(1));
    let mut enqueued_total = 0usize;
    let reconnect_request = reconnect_request_path(state_root);
    let mut force_reconnect_once = reconnect_request.exists();
    if force_reconnect_once {
        let _ = fs::remove_file(&reconnect_request);
    }

    loop {
        if should_stop(stop) || deadline_reached(deadline) {
            break;
        }

        health.last_reconnect = Some(super::now_secs());
        let url = match runtime.api.open_socket_connection_url() {
            Ok(value) => value,
            Err(err @ SlackError::RateLimited { .. }) => {
                health.connected = false;
                health.last_error = Some(err.to_string());
                save_health(state_root, profile_id, &health);
                return Err(err);
            }
            Err(err) => {
                let class = classify_socket_failure(&err.to_string());
                let message =
                    format_socket_error("socket url open failed", &err.to_string(), class);
                health.connected = false;
                health.last_error = Some(message.clone());
                save_health(state_root, profile_id, &health);
                if class == RetryClass::NonRetryable {
                    return Err(SlackError::ApiRequest(message));
                }
                if !sleep_reconnect(reconnect_backoff, stop, deadline) {
                    break;
                }
                continue;
            }
        };

        let (mut socket, _) = match connect(url.as_str()) {
            Ok(connection) => connection,
            Err(err) => {
                let class = classify_socket_failure(&err.to_string());
                let message = format_socket_error("socket connect failed", &err.to_string(), class);
                health.connected = false;
                health.last_error = Some(message);
                save_health(state_root, profile_id, &health);
                if class == RetryClass::NonRetryable {
                    return Err(SlackError::ApiRequest(
                        health.last_error.clone().unwrap_or_default(),
                    ));
                }
                if !sleep_reconnect(reconnect_backoff, stop, deadline) {
                    break;
                }
                continue;
            }
        };
        health.connected = true;
        health.last_error = None;
        save_health(state_root, profile_id, &health);
        if let Err(err) = set_socket_nonblocking(&mut socket) {
            health.connected = false;
            health.last_error = Some(err.to_string());
            save_health(state_root, profile_id, &health);
            return Err(err);
        }

        let control = LoopControl { deadline, stop };
        let (enqueued, outcome) = process_single_connection(
            state_root,
            &mut socket,
            queue_paths,
            profile_id,
            runtime,
            control,
            &mut health,
        )?;
        enqueued_total += enqueued;
        health.connected = false;
        save_health(state_root, profile_id, &health);

        if force_reconnect_once {
            force_reconnect_once = false;
            if !sleep_reconnect(reconnect_backoff, stop, deadline) {
                break;
            }
            continue;
        }

        match outcome {
            SocketLoopOutcome::StopRequested | SocketLoopOutcome::DeadlineReached => break,
            SocketLoopOutcome::ReconnectRequested | SocketLoopOutcome::Disconnected => {
                if !sleep_reconnect(reconnect_backoff, stop, deadline) {
                    break;
                }
            }
        }
    }

    if health.connected {
        health.connected = false;
        save_health(state_root, profile_id, &health);
    }

    Ok(enqueued_total)
}

fn should_stop(stop: Option<&AtomicBool>) -> bool {
    stop.map(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}

fn deadline_reached(deadline: Option<Instant>) -> bool {
    deadline
        .map(|value| Instant::now() >= value)
        .unwrap_or(false)
}

fn sleep_reconnect(
    backoff: Duration,
    stop: Option<&AtomicBool>,
    deadline: Option<Instant>,
) -> bool {
    let jittered = backoff + reconnect_jitter(backoff);
    let mut remaining = jittered;
    while remaining > Duration::ZERO {
        if should_stop(stop) || deadline_reached(deadline) {
            return false;
        }
        let step = remaining.min(Duration::from_millis(25));
        thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
    !should_stop(stop) && !deadline_reached(deadline)
}

fn reconnect_jitter(backoff: Duration) -> Duration {
    let ceiling = backoff.min(Duration::from_millis(500)).as_millis() as u64;
    if ceiling == 0 {
        return Duration::ZERO;
    }
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos() as u64)
        .unwrap_or(0);
    Duration::from_millis(seed % (ceiling + 1))
}

fn classify_socket_failure(message: &str) -> RetryClass {
    let lower = message.to_ascii_lowercase();
    if [
        "invalid_auth",
        "not_authed",
        "token_revoked",
        "account_inactive",
        "missing_scope",
        "403",
        "401",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        RetryClass::NonRetryable
    } else {
        RetryClass::Retryable
    }
}

fn format_socket_error(context: &str, detail: &str, class: RetryClass) -> String {
    let class = match class {
        RetryClass::Retryable => "retryable",
        RetryClass::NonRetryable => "non_retryable",
    };
    format!("{context} ({class}): {detail}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SocketLoopOutcome {
    Disconnected,
    ReconnectRequested,
    StopRequested,
    DeadlineReached,
}

#[derive(Clone, Copy)]
struct LoopControl<'a> {
    deadline: Option<Instant>,
    stop: Option<&'a AtomicBool>,
}

fn process_single_connection(
    state_root: &Path,
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
    control: LoopControl<'_>,
    health: &mut SocketHealth,
) -> Result<(usize, SocketLoopOutcome), SlackError> {
    let reconnect_request = reconnect_request_path(state_root);

    let (sender, receiver): (SyncSender<QueueCandidate>, Receiver<QueueCandidate>) =
        mpsc::sync_channel(SOCKET_BUFFER_CAPACITY);
    let enqueued = Arc::new(AtomicUsize::new(0));
    let enqueued_for_worker = Arc::clone(&enqueued);
    let queue_paths = queue_paths.clone();
    let profile = runtime.profile.clone();
    let profile_id = profile_id.to_string();
    let worker = thread::spawn(move || {
        while let Ok(candidate) = receiver.recv() {
            if enqueue_incoming(
                &queue_paths,
                profile_id.as_str(),
                &profile,
                candidate.channel.as_str(),
                &candidate.message,
            )
            .unwrap_or(false)
            {
                enqueued_for_worker.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    let mut outcome = SocketLoopOutcome::Disconnected;
    loop {
        if should_stop(control.stop) {
            outcome = SocketLoopOutcome::StopRequested;
            break;
        }
        if deadline_reached(control.deadline) {
            outcome = SocketLoopOutcome::DeadlineReached;
            break;
        }
        if reconnect_request.exists() {
            let _ = fs::remove_file(&reconnect_request);
            outcome = SocketLoopOutcome::ReconnectRequested;
            break;
        }

        match socket.read() {
            Ok(Message::Text(text)) => {
                handle_socket_text(socket, runtime, text.as_str(), &sender, health);
            }
            Ok(Message::Binary(_)) => {}
            Ok(Message::Ping(payload)) => {
                let _ = socket.send(Message::Pong(payload));
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Frame(_)) => {}
            Ok(Message::Close(_)) => break,
            Err(tungstenite::Error::Io(err))
                if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut =>
            {
                thread::sleep(SOCKET_IDLE_SLEEP);
            }
            Err(tungstenite::Error::ConnectionClosed) => break,
            Err(err) => {
                let class = classify_socket_failure(&err.to_string());
                health.last_error = Some(format_socket_error(
                    "socket read failed",
                    &err.to_string(),
                    class,
                ));
                break;
            }
        }
    }

    drop(sender);
    let _ = worker.join();
    let _ = socket.close(None);
    Ok((enqueued.load(Ordering::Relaxed), outcome))
}

#[derive(Debug)]
struct QueueCandidate {
    channel: String,
    message: SlackMessage,
}

fn handle_socket_text(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    runtime: &SlackProfileRuntime,
    text: &str,
    sender: &SyncSender<QueueCandidate>,
    health: &mut SocketHealth,
) {
    let envelope = match serde_json::from_str::<SocketEnvelope>(text) {
        Ok(value) => value,
        Err(_) => return,
    };

    if let Some(envelope_id) = envelope.envelope_id {
        let ack = json!({ "envelope_id": envelope_id }).to_string();
        let _ = socket.send(Message::Text(ack));
    }

    let Some(event) = envelope.payload.and_then(|payload| payload.event) else {
        return;
    };

    if !should_enqueue_socket_event(&event, &runtime.profile, &runtime.allowlist) {
        return;
    }

    let message = SlackMessage {
        ts: event.ts.clone(),
        thread_ts: event.thread_ts.clone(),
        text: event.text.clone(),
        user: event.user,
        subtype: event.subtype,
        bot_id: event.bot_id,
        reply_count: None,
    };

    match sender.try_send(QueueCandidate {
        channel: event.channel,
        message,
    }) {
        Ok(()) => {
            health.last_event_ts = Some(event.ts);
        }
        Err(TrySendError::Full(_)) => {}
        Err(TrySendError::Disconnected(_)) => {}
    }
}

fn should_enqueue_socket_event(
    event: &SocketEvent,
    profile: &ChannelProfile,
    allowlist: &BTreeSet<String>,
) -> bool {
    if event.r#type != "message" && event.r#type != "app_mention" {
        return false;
    }
    if event.channel.trim().is_empty() || event.ts.trim().is_empty() {
        return false;
    }
    if event.user.is_none() || event.bot_id.is_some() || event.subtype.is_some() {
        return false;
    }
    let Some(channel_type) = resolve_channel_type(event) else {
        return false;
    };
    if channel_type != "im" && channel_type != "channel" && channel_type != "group" {
        return false;
    }
    if channel_type != "im"
        && !should_accept_channel_message(
            profile,
            allowlist,
            &event.channel,
            event.text.as_deref().unwrap_or(""),
            &event.ts,
            event.thread_ts.as_deref(),
        )
    {
        return false;
    }
    true
}

fn resolve_channel_type(event: &SocketEvent) -> Option<&str> {
    event
        .channel_type
        .as_deref()
        .or_else(|| infer_channel_type(event.channel.as_str()))
}

fn infer_channel_type(channel_id: &str) -> Option<&'static str> {
    let mut chars = channel_id.chars();
    match chars.next() {
        Some('D') => Some("im"),
        Some('C') => Some("channel"),
        Some('G') => Some("group"),
        _ => None,
    }
}

fn set_socket_nonblocking(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
) -> Result<(), SlackError> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream.set_nonblocking(true),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_nonblocking(true),
        _ => Ok(()),
    }
    .map_err(|err| SlackError::ApiRequest(format!("failed to configure socket mode stream: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelKind, ThreadResponseMode};

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

    fn base_event(channel_type: &str) -> SocketEvent {
        SocketEvent {
            r#type: "message".to_string(),
            channel: "C001".to_string(),
            channel_type: Some(channel_type.to_string()),
            user: Some("U123".to_string()),
            bot_id: None,
            subtype: None,
            text: Some("hello".to_string()),
            ts: "200.0".to_string(),
            thread_ts: None,
        }
    }

    #[test]
    fn accepts_channel_or_group_thread_replies() {
        let mut channel_event = base_event("channel");
        channel_event.thread_ts = Some("100.0".to_string());
        assert!(should_enqueue_socket_event(
            &channel_event,
            &profile(Some("UAPP")),
            &BTreeSet::new()
        ));

        let mut group_event = base_event("group");
        group_event.thread_ts = Some("100.0".to_string());
        assert!(should_enqueue_socket_event(
            &group_event,
            &profile(Some("UAPP")),
            &BTreeSet::new()
        ));
    }

    #[test]
    fn non_thread_channel_messages_are_accepted_for_opportunistic_policy() {
        let channel_event = base_event("channel");
        assert!(should_enqueue_socket_event(
            &channel_event,
            &profile(Some("UAPP")),
            &BTreeSet::new()
        ));
    }

    #[test]
    fn app_mentions_without_channel_type_are_accepted() {
        let mut event = base_event("channel");
        event.r#type = "app_mention".to_string();
        event.channel_type = None;
        event.text = Some("<@UAPP> run this".to_string());
        assert!(should_enqueue_socket_event(
            &event,
            &profile(Some("UAPP")),
            &BTreeSet::new()
        ));
    }

    #[test]
    fn socket_error_classification_marks_auth_errors_non_retryable() {
        assert_eq!(
            classify_socket_failure("invalid_auth while opening socket"),
            RetryClass::NonRetryable
        );
        assert_eq!(
            classify_socket_failure("temporary dns resolution failure"),
            RetryClass::Retryable
        );
    }

    #[test]
    fn socket_error_message_includes_details_and_classification() {
        let message = format_socket_error(
            "socket connect failed",
            "tls handshake eof",
            RetryClass::Retryable,
        );
        assert!(message.contains("socket connect failed"));
        assert!(message.contains("tls handshake eof"));
        assert!(message.contains("retryable"));
    }
}
