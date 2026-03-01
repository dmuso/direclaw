use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadContextLimits {
    pub max_turns: usize,
    pub max_chars: usize,
}

impl Default for ThreadContextLimits {
    fn default() -> Self {
        Self {
            max_turns: 8,
            max_chars: 6000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TurnDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnRecord {
    timestamp: i64,
    direction: TurnDirection,
    sender_id: String,
    message: String,
    message_id: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    workflow_run_id: Option<String>,
    #[serde(default)]
    workflow_step_id: Option<String>,
}

fn sanitize_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "_".to_string()
    } else {
        out
    }
}

fn context_path(state_root: &Path, channel_profile_id: &str, conversation_id: &str) -> PathBuf {
    state_root
        .join("orchestrator/conversations")
        .join(sanitize_component(channel_profile_id))
        .join(format!("{}.jsonl", sanitize_component(conversation_id)))
}

fn append_turn(
    state_root: &Path,
    channel_profile_id: &str,
    conversation_id: &str,
    turn: &TurnRecord,
) -> std::io::Result<()> {
    let path = context_path(state_root, channel_profile_id, conversation_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    let line = serde_json::to_string(turn).map_err(std::io::Error::other)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn append_inbound_turn(
    state_root: &Path,
    channel_profile_id: &str,
    conversation_id: &str,
    message_id: &str,
    timestamp: i64,
    sender_id: &str,
    message: &str,
) -> std::io::Result<()> {
    if message.trim().is_empty() {
        return Ok(());
    }
    append_turn(
        state_root,
        channel_profile_id,
        conversation_id,
        &TurnRecord {
            timestamp,
            direction: TurnDirection::Inbound,
            sender_id: sender_id.to_string(),
            message: message.to_string(),
            message_id: message_id.to_string(),
            agent: None,
            workflow_run_id: None,
            workflow_step_id: None,
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn append_outbound_turn(
    state_root: &Path,
    channel_profile_id: &str,
    conversation_id: &str,
    message_id: &str,
    timestamp: i64,
    agent: &str,
    message: &str,
    workflow_run_id: Option<&str>,
    workflow_step_id: Option<&str>,
) -> std::io::Result<()> {
    if message.trim().is_empty() {
        return Ok(());
    }
    append_turn(
        state_root,
        channel_profile_id,
        conversation_id,
        &TurnRecord {
            timestamp,
            direction: TurnDirection::Outbound,
            sender_id: "assistant".to_string(),
            message: message.to_string(),
            message_id: message_id.to_string(),
            agent: Some(agent.to_string()),
            workflow_run_id: workflow_run_id.map(|value| value.to_string()),
            workflow_step_id: workflow_step_id.map(|value| value.to_string()),
        },
    )
}

fn render_turn(turn: &TurnRecord) -> String {
    let who = match turn.direction {
        TurnDirection::Inbound => "user",
        TurnDirection::Outbound => "assistant",
    };
    let mut line = format!("[{who}] {}", turn.message.trim());
    if let Some(run_id) = turn.workflow_run_id.as_deref() {
        line.push_str(&format!(" (run_id={run_id})"));
    }
    if let Some(step_id) = turn.workflow_step_id.as_deref() {
        line.push_str(&format!(" (step_id={step_id})"));
    }
    line
}

pub fn render_recent_thread_context(
    state_root: &Path,
    channel_profile_id: &str,
    conversation_id: &str,
    limits: ThreadContextLimits,
) -> std::io::Result<Option<String>> {
    if limits.max_turns == 0 || limits.max_chars == 0 {
        return Ok(None);
    }

    let path = context_path(state_root, channel_profile_id, conversation_id);
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };

    let reader = BufReader::new(file);
    let mut turns = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(turn) = serde_json::from_str::<TurnRecord>(&line) {
            turns.push(turn);
        }
    }
    if turns.is_empty() {
        return Ok(None);
    }

    turns.sort_by(|left, right| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left.message_id.cmp(&right.message_id))
    });
    if turns.len() > limits.max_turns {
        let keep_from = turns.len() - limits.max_turns;
        turns = turns.split_off(keep_from);
    }

    let mut selected_lines = Vec::<String>::new();
    let mut used = 0usize;
    for line in turns.iter().rev().map(render_turn) {
        let line_len = line.chars().count();
        let sep = if selected_lines.is_empty() { 0 } else { 1 };
        if used + sep + line_len > limits.max_chars {
            break;
        }
        used += sep + line_len;
        selected_lines.push(line);
    }
    selected_lines.reverse();
    if selected_lines.is_empty() {
        return Ok(None);
    }
    Ok(Some(selected_lines.join("\n")))
}
