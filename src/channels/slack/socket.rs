use super::api::SlackMessage;
use super::ingest::enqueue_incoming;
use super::{SlackError, SlackProfileRuntime};
use crate::queue::QueuePaths;
use serde::Deserialize;
use serde_json::json;
use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const SOCKET_POLL_WINDOW: Duration = Duration::from_millis(1500);
const SOCKET_IDLE_SLEEP: Duration = Duration::from_millis(40);

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
    _state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
) -> Result<usize, SlackError> {
    let url = runtime.api.open_socket_connection_url()?;
    let (mut socket, _) = match connect(url.as_str()) {
        Ok(connection) => connection,
        Err(_) => return Ok(0),
    };
    set_socket_nonblocking(&mut socket)?;

    let started_at = Instant::now();
    let mut enqueued = 0usize;
    while started_at.elapsed() < SOCKET_POLL_WINDOW {
        match socket.read() {
            Ok(Message::Text(text)) => {
                enqueued +=
                    handle_socket_text(&mut socket, queue_paths, profile_id, text.as_str())?;
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
    let _ = socket.close(None);
    Ok(enqueued)
}

fn handle_socket_text(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    queue_paths: &QueuePaths,
    profile_id: &str,
    text: &str,
) -> Result<usize, SlackError> {
    let envelope = match serde_json::from_str::<SocketEnvelope>(text) {
        Ok(value) => value,
        Err(_) => return Ok(0),
    };

    if let Some(envelope_id) = envelope.envelope_id {
        let ack = json!({ "envelope_id": envelope_id }).to_string();
        let _ = socket.send(Message::Text(ack));
    }

    let Some(event) = envelope.payload.and_then(|payload| payload.event) else {
        return Ok(0);
    };

    if event.r#type != "message" {
        return Ok(0);
    }
    if event.channel.trim().is_empty()
        || event.ts.trim().is_empty()
        || event.channel_type.as_deref() != Some("im")
    {
        return Ok(0);
    }
    if event.user.is_none() || event.bot_id.is_some() || event.subtype.is_some() {
        return Ok(0);
    }

    let message = SlackMessage {
        ts: event.ts.clone(),
        thread_ts: event.thread_ts.clone(),
        text: event.text.clone(),
        user: event.user,
        subtype: event.subtype,
        bot_id: event.bot_id,
    };

    if enqueue_incoming(queue_paths, profile_id, &event.channel, &message)? {
        return Ok(1);
    }
    Ok(0)
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
