use crate::config::{ChannelKind, ChannelProfile, Settings};
use crate::queue::{sorted_outgoing_paths, IncomingMessage, OutgoingMessage, QueuePaths};
use crate::runtime::drain_queue_once;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CHAT_EXIT_COMMANDS: &[&str] = &["/exit", "exit", "quit"];
const CHAT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const CHAT_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub fn run_local_chat_session_stdio(
    state_root: &Path,
    settings: &Settings,
    profile_id: &str,
) -> Result<String, String> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    run_local_chat_session(state_root, settings, profile_id, &mut input, &mut output)
}

pub fn run_local_chat_session<R: BufRead, W: Write>(
    state_root: &Path,
    settings: &Settings,
    profile_id: &str,
    input: &mut R,
    output: &mut W,
) -> Result<String, String> {
    let profile = settings
        .channel_profiles
        .get(profile_id)
        .ok_or_else(|| format!("unknown channel profile `{profile_id}`"))?;
    if profile.channel != ChannelKind::Local {
        return Err(format!(
            "chat requires a local channel profile, `{profile_id}` uses `{}`",
            profile.channel
        ));
    }

    let queue_paths = QueuePaths::from_state_root(state_root);
    let conversation_id = format!("chat-{}", now_nanos());
    let session = ChatSession {
        state_root,
        settings,
        queue_paths: &queue_paths,
        profile_id,
        profile,
        conversation_id: &conversation_id,
    };
    run_chat_repl(input, output, &session)?;

    Ok(format!("chat ended\nconversation_id={conversation_id}"))
}

struct ChatSession<'a> {
    state_root: &'a Path,
    settings: &'a Settings,
    queue_paths: &'a QueuePaths,
    profile_id: &'a str,
    profile: &'a ChannelProfile,
    conversation_id: &'a str,
}

fn run_chat_repl<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    session: &ChatSession<'_>,
) -> Result<(), String> {
    writeln!(
        output,
        "chat profile={} conversation_id={}",
        session.profile_id, session.conversation_id
    )
    .map_err(|e| format!("failed to write chat output: {e}"))?;
    writeln!(output, "type `/exit` to quit")
        .map_err(|e| format!("failed to write chat output: {e}"))?;

    loop {
        write!(output, "you> ").map_err(|e| format!("failed to write chat prompt: {e}"))?;
        output
            .flush()
            .map_err(|e| format!("failed to flush chat prompt: {e}"))?;

        let mut line = String::new();
        let read = input
            .read_line(&mut line)
            .map_err(|e| format!("failed to read chat input: {e}"))?;
        if read == 0 {
            break;
        }

        let message = line.trim();
        if message.is_empty() {
            continue;
        }
        if is_chat_exit_command(message) {
            break;
        }

        let message_id = enqueue_chat_message(
            session.queue_paths,
            session.profile_id,
            session.profile,
            session.conversation_id,
            message,
        )?;
        let _ = drain_queue_once(session.state_root, session.settings, 1)
            .map_err(|e| format!("chat processing failed: {e}"))?;

        match wait_for_outgoing_message(session.queue_paths, &message_id, CHAT_RESPONSE_TIMEOUT)? {
            Some(response) => {
                writeln!(output, "assistant> {}", response.message)
                    .map_err(|e| format!("failed to write chat output: {e}"))?;
                output
                    .flush()
                    .map_err(|e| format!("failed to flush chat output: {e}"))?;
            }
            None => {
                writeln!(
                    output,
                    "assistant> timed out waiting for response (message_id={message_id})"
                )
                .map_err(|e| format!("failed to write chat timeout output: {e}"))?;
                output
                    .flush()
                    .map_err(|e| format!("failed to flush chat output: {e}"))?;
            }
        }
    }

    Ok(())
}

fn is_chat_exit_command(message: &str) -> bool {
    CHAT_EXIT_COMMANDS
        .iter()
        .any(|command| message.eq_ignore_ascii_case(command))
}

fn enqueue_chat_message(
    queue_paths: &QueuePaths,
    profile_id: &str,
    profile: &ChannelProfile,
    conversation_id: &str,
    message: &str,
) -> Result<String, String> {
    let msg_id = format!("msg-{}", now_nanos());
    let incoming = IncomingMessage {
        channel: profile.channel.to_string(),
        channel_profile_id: Some(profile_id.to_string()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: message.to_string(),
        timestamp: now_secs(),
        message_id: msg_id.clone(),
        conversation_id: Some(conversation_id.to_string()),
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    let queue_path = queue_paths
        .incoming
        .join(format!("{}.json", incoming.message_id));
    let body = serde_json::to_vec_pretty(&incoming)
        .map_err(|e| format!("failed to encode queue message: {e}"))?;
    fs::write(&queue_path, body)
        .map_err(|e| format!("failed to write {}: {e}", queue_path.display()))?;
    Ok(msg_id)
}

fn wait_for_outgoing_message(
    queue_paths: &QueuePaths,
    message_id: &str,
    timeout: Duration,
) -> Result<Option<OutgoingMessage>, String> {
    let started = Instant::now();
    while started.elapsed() <= timeout {
        for path in sorted_outgoing_paths(queue_paths)
            .map_err(|e| format!("failed to read {}: {e}", queue_paths.outgoing.display()))?
        {
            let raw = fs::read_to_string(&path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            let outgoing: OutgoingMessage = serde_json::from_str(&raw)
                .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
            if outgoing.message_id != message_id {
                continue;
            }
            fs::remove_file(&path)
                .map_err(|e| format!("failed to remove {}: {e}", path.display()))?;
            return Ok(Some(outgoing));
        }
        thread::sleep(CHAT_POLL_INTERVAL);
    }
    Ok(None)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{is_chat_exit_command, wait_for_outgoing_message};
    use crate::queue::{OutgoingMessage, QueuePaths};
    use std::fs;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn chat_exit_commands_are_case_insensitive() {
        assert!(is_chat_exit_command("/exit"));
        assert!(is_chat_exit_command("EXIT"));
        assert!(is_chat_exit_command("Quit"));
        assert!(!is_chat_exit_command("continue"));
    }

    #[test]
    fn wait_for_outgoing_message_consumes_matching_message_file() {
        let dir = tempdir().expect("tempdir");
        let queue = QueuePaths::from_state_root(dir.path());
        fs::create_dir_all(&queue.outgoing).expect("outgoing");

        let matching = OutgoingMessage {
            channel: "local".to_string(),
            channel_profile_id: Some("local-default".to_string()),
            sender: "cli".to_string(),
            message: "ok".to_string(),
            original_message: "hello".to_string(),
            timestamp: 1,
            message_id: "msg-1".to_string(),
            agent: "orchestrator".to_string(),
            conversation_id: Some("chat-1".to_string()),
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        };

        let other = OutgoingMessage {
            message_id: "msg-2".to_string(),
            ..matching.clone()
        };

        let other_path = queue.outgoing.join("local_msg-2_1.json");
        let match_path = queue.outgoing.join("local_msg-1_1.json");
        fs::write(
            &other_path,
            serde_json::to_string(&other).expect("serialize other"),
        )
        .expect("write other");
        fs::write(
            &match_path,
            serde_json::to_string(&matching).expect("serialize matching"),
        )
        .expect("write matching");

        let received =
            wait_for_outgoing_message(&queue, "msg-1", Duration::from_secs(1)).expect("receive");
        assert_eq!(received.expect("message").message, "ok");
        assert!(other_path.exists(), "non-matching message should remain");
        assert!(
            !match_path.exists(),
            "matching message should be removed after consume"
        );
    }
}
