use crate::app::command_support::{ensure_runtime_root, load_settings, now_nanos, now_secs};
use crate::channels::slack;
use crate::queue::IncomingMessage;
use std::fs;

pub fn cmd_send(args: &[String]) -> Result<String, String> {
    if args.len() < 2 {
        return Err("usage: send <channel_profile_id> <message>".to_string());
    }
    let settings = load_settings()?;
    let profile_id = args[0].clone();
    let profile = settings
        .channel_profiles
        .get(&profile_id)
        .ok_or_else(|| format!("unknown channel profile `{profile_id}`"))?;
    let message = args[1..].join(" ");

    let _paths = ensure_runtime_root()?;
    let ts = now_secs();
    let msg_id = format!("msg-{}", now_nanos());
    let incoming = IncomingMessage {
        channel: profile.channel.to_string(),
        channel_profile_id: Some(profile_id.clone()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message,
        timestamp: ts,
        message_id: msg_id.clone(),
        conversation_id: None,
        is_direct: true,
        is_thread_reply: false,
        is_mentioned: false,
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let runtime_root = settings
        .resolve_channel_profile_runtime_root(&profile_id)
        .map_err(|e| e.to_string())?;
    let queue_dir = runtime_root.join("queue/incoming");
    fs::create_dir_all(&queue_dir)
        .map_err(|e| format!("failed to create {}: {e}", queue_dir.display()))?;
    let queue_path = queue_dir.join(format!("{}.json", incoming.message_id));
    let body = serde_json::to_vec_pretty(&incoming)
        .map_err(|e| format!("failed to encode queue message: {e}"))?;
    fs::write(&queue_path, body)
        .map_err(|e| format!("failed to write {}: {e}", queue_path.display()))?;
    Ok(format!("queued\nmessage_id={msg_id}"))
}

pub fn cmd_channels(args: &[String]) -> Result<String, String> {
    if args.len() == 1 && args[0] == "reset" {
        let paths = ensure_runtime_root()?;
        let channels_dir = paths.root.join("channels");
        if channels_dir.exists() {
            fs::remove_dir_all(&channels_dir)
                .map_err(|e| format!("failed to reset {}: {e}", channels_dir.display()))?;
        }
        fs::create_dir_all(&channels_dir)
            .map_err(|e| format!("failed to create {}: {e}", channels_dir.display()))?;
        return Ok("channels reset complete".to_string());
    }
    if args.len() == 2 && args[0] == "slack" && args[1] == "sync" {
        let paths = ensure_runtime_root()?;
        let settings = load_settings()?;
        let report = slack::sync_once(&paths.root, &settings).map_err(|e| e.to_string())?;
        return Ok(format!(
            "slack sync complete\nprofiles_processed={}\ninbound_enqueued={}\noutbound_messages_sent={}",
            report.profiles_processed, report.inbound_enqueued, report.outbound_messages_sent
        ));
    }
    if args.len() == 3 && args[0] == "slack" && args[1] == "socket" && args[2] == "status" {
        let paths = ensure_runtime_root()?;
        let settings = load_settings()?;
        let health = slack::socket_health(&paths.root, &settings);
        if health.is_empty() {
            return Ok("connected=false".to_string());
        }
        let mut lines = Vec::new();
        for item in health {
            lines.push(format!("profile={}", item.profile_id));
            lines.push(format!("connected={}", item.connected));
            lines.push(format!(
                "last_event_ts={}",
                item.last_event_ts.unwrap_or_else(|| "none".to_string())
            ));
            lines.push(format!(
                "last_reconnect={}",
                item.last_reconnect
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ));
            lines.push(format!(
                "last_error={}",
                item.last_error.unwrap_or_else(|| "none".to_string())
            ));
        }
        return Ok(lines.join("\n"));
    }
    if args.len() == 3 && args[0] == "slack" && args[1] == "socket" && args[2] == "reconnect" {
        let paths = ensure_runtime_root()?;
        slack::request_socket_reconnect(&paths.root).map_err(|e| e.to_string())?;
        return Ok("reconnect_requested=true".to_string());
    }
    if args.len() == 3 && args[0] == "slack" && args[1] == "backfill" && args[2] == "run" {
        let paths = ensure_runtime_root()?;
        let settings = load_settings()?;
        let report =
            slack::sync_backfill_once(&paths.root, &settings).map_err(|e| e.to_string())?;
        return Ok(format!(
            "slack backfill complete\nprofiles_processed={}\ninbound_enqueued={}\noutbound_messages_sent={}",
            report.profiles_processed, report.inbound_enqueued, report.outbound_messages_sent
        ));
    }
    Err(
        "usage: channels reset | channels slack sync | channels slack socket <status|reconnect> | channels slack backfill run".to_string(),
    )
}
