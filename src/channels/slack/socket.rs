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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const SOCKET_IDLE_SLEEP: Duration = Duration::from_millis(40);
const SOCKET_BUFFER_CAPACITY: usize = 256;

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
    let mut health = load_health(state_root, profile_id);
    let deadline = Instant::now() + Duration::from_millis(idle_timeout_ms.max(1));
    let reconnect_backoff = Duration::from_millis(reconnect_backoff_ms.max(1));
    let mut enqueued_total = 0usize;

    let reconnect_request = reconnect_request_path(state_root);
    let force_reconnect = reconnect_request.exists();
    if force_reconnect {
        let _ = fs::remove_file(&reconnect_request);
    }

    while Instant::now() < deadline {
        health.last_reconnect = Some(super::now_secs());
        health.connected = false;
        let url = runtime.api.open_socket_connection_url()?;
        let (mut socket, _) = match connect(url.as_str()) {
            Ok(connection) => connection,
            Err(_) => {
                health.last_error = Some("socket connect failed".to_string());
                save_health(state_root, profile_id, &health);
                thread::sleep(reconnect_backoff);
                continue;
            }
        };
        health.connected = true;
        health.last_error = None;
        save_health(state_root, profile_id, &health);
        set_socket_nonblocking(&mut socket)?;

        let enqueued = process_single_connection(
            &mut socket,
            queue_paths,
            profile_id,
            runtime,
            deadline,
            &mut health,
        )?;
        enqueued_total += enqueued;
        save_health(state_root, profile_id, &health);
        if !force_reconnect {
            break;
        }
        thread::sleep(reconnect_backoff);
    }
    health.connected = false;
    save_health(state_root, profile_id, &health);

    Ok(enqueued_total)
}

fn process_single_connection(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
    deadline: Instant,
    health: &mut SocketHealth,
) -> Result<usize, SlackError> {
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

    while Instant::now() < deadline {
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
            Err(_) => break,
        }
    }

    drop(sender);
    let _ = worker.join();
    let _ = socket.close(None);
    Ok(enqueued.load(Ordering::Relaxed))
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
    if event.r#type != "message" {
        return false;
    }
    if event.channel.trim().is_empty() || event.ts.trim().is_empty() {
        return false;
    }
    if event.user.is_none() || event.bot_id.is_some() || event.subtype.is_some() {
        return false;
    }
    let Some(channel_type) = event.channel_type.as_deref() else {
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
}
