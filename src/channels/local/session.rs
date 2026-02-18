use crate::config::{ChannelKind, ChannelProfile, Settings};
use crate::queue::{sorted_outgoing_paths, IncomingMessage, OutgoingMessage, QueuePaths};
use crate::runtime::drain_queue_once;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const CHAT_EXIT_COMMANDS: &[&str] = &["/exit", "exit", "quit"];
pub const CHAT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const CHAT_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct LocalChatSession {
    pub state_root: PathBuf,
    pub settings: Settings,
    pub queue_paths: QueuePaths,
    pub profile_id: String,
    pub profile: ChannelProfile,
    pub conversation_id: String,
}

pub fn create_local_chat_session(
    state_root: &Path,
    settings: &Settings,
    profile_id: &str,
) -> Result<LocalChatSession, String> {
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

    Ok(LocalChatSession {
        state_root: state_root.to_path_buf(),
        settings: settings.clone(),
        queue_paths: QueuePaths::from_state_root(
            &settings
                .resolve_channel_profile_runtime_root(profile_id)
                .map_err(|err| err.to_string())?,
        ),
        profile_id: profile_id.to_string(),
        profile: profile.clone(),
        conversation_id: format!("chat-{}", now_nanos()),
    })
}

pub fn is_chat_exit_command(message: &str) -> bool {
    CHAT_EXIT_COMMANDS
        .iter()
        .any(|command| message.eq_ignore_ascii_case(command))
}

pub fn enqueue_chat_message(session: &LocalChatSession, message: &str) -> Result<String, String> {
    let msg_id = format!("msg-{}", now_nanos());
    let incoming = IncomingMessage {
        channel: session.profile.channel.to_string(),
        channel_profile_id: Some(session.profile_id.clone()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: message.to_string(),
        timestamp: now_secs(),
        message_id: msg_id.clone(),
        conversation_id: Some(session.conversation_id.clone()),
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    fs::create_dir_all(&session.queue_paths.incoming).map_err(|e| {
        format!(
            "failed to prepare {}: {e}",
            session.queue_paths.incoming.display()
        )
    })?;
    let queue_path = session
        .queue_paths
        .incoming
        .join(format!("{}.json", incoming.message_id));
    let body = serde_json::to_vec_pretty(&incoming)
        .map_err(|e| format!("failed to encode queue message: {e}"))?;
    fs::write(&queue_path, body)
        .map_err(|e| format!("failed to write {}: {e}", queue_path.display()))?;
    Ok(msg_id)
}

pub fn process_message(
    session: &LocalChatSession,
    message_id: &str,
) -> Result<Vec<OutgoingMessage>, String> {
    fs::create_dir_all(&session.queue_paths.outgoing).map_err(|e| {
        format!(
            "failed to prepare {}: {e}",
            session.queue_paths.outgoing.display()
        )
    })?;
    let _ = drain_queue_once(&session.state_root, &session.settings, 1)
        .map_err(|e| format!("chat processing failed: {e}"))?;
    wait_for_outgoing_messages(
        &session.queue_paths,
        message_id,
        CHAT_RESPONSE_TIMEOUT,
        || Ok(()),
    )
}

pub fn wait_for_outgoing_messages<F>(
    queue_paths: &QueuePaths,
    message_id: &str,
    timeout: Duration,
    mut on_poll: F,
) -> Result<Vec<OutgoingMessage>, String>
where
    F: FnMut() -> Result<(), String>,
{
    let settle_window = Duration::from_millis(300);
    let started = Instant::now();
    let mut found = Vec::new();
    let mut first_received_at: Option<Instant> = None;

    while started.elapsed() <= timeout {
        on_poll()?;
        let mut matched_on_this_poll = false;
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
            matched_on_this_poll = true;
            if first_received_at.is_none() {
                first_received_at = Some(Instant::now());
            }
            found.push(outgoing);
        }

        if first_received_at.is_some()
            && !matched_on_this_poll
            && first_received_at
                .map(|received_at| received_at.elapsed() >= settle_window)
                .unwrap_or(false)
        {
            break;
        }
        thread::sleep(CHAT_POLL_INTERVAL);
    }

    Ok(found)
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
    use super::{is_chat_exit_command, wait_for_outgoing_messages};
    use crate::channels::local::session::create_local_chat_session;
    use crate::queue::{OutgoingMessage, QueuePaths};
    use std::fs;
    use std::path::PathBuf;
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
    fn wait_for_outgoing_messages_consumes_all_matching_message_files() {
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
            target_ref: None,
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

        let match_path_2 = queue.outgoing.join("local_msg-1_2.json");
        fs::write(
            &match_path_2,
            serde_json::to_string(&matching).expect("serialize matching 2"),
        )
        .expect("write matching 2");

        let received =
            wait_for_outgoing_messages(&queue, "msg-1", Duration::from_secs(1), || Ok(()))
                .expect("receive");
        assert_eq!(received.len(), 2);
        assert_eq!(received[0].message, "ok");
        assert_eq!(received[1].message, "ok");
        assert!(other_path.exists(), "non-matching message should remain");
        assert!(
            !match_path.exists(),
            "first matching message should be removed after consume"
        );
        assert!(
            !match_path_2.exists(),
            "second matching message should be removed after consume"
        );
    }

    #[test]
    fn create_local_chat_session_scopes_queue_paths_to_orchestrator_workspace() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join(".direclaw");
        let orchestrator_workspace = temp.path().join("workspaces/eng");
        fs::create_dir_all(&orchestrator_workspace).expect("orchestrator workspace");
        let settings: crate::config::Settings = serde_yaml::from_str(&format!(
            r#"
workspaces_path: {workspaces}
shared_workspaces: {{}}
orchestrators:
  eng:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles:
  local-default:
    channel: local
    orchestrator_id: eng
monitoring: {{}}
channels: {{}}
"#,
            workspaces = temp.path().join("workspaces").display(),
            orchestrator_workspace = orchestrator_workspace.display(),
        ))
        .expect("settings");

        let session =
            create_local_chat_session(&state_root, &settings, "local-default").expect("session");
        assert_eq!(
            session.queue_paths.incoming,
            PathBuf::from(&orchestrator_workspace).join("queue/incoming")
        );
    }
}
