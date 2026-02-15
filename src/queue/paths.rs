use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuePaths {
    pub incoming: PathBuf,
    pub processing: PathBuf,
    pub outgoing: PathBuf,
}

impl QueuePaths {
    pub fn from_state_root(state_root: &Path) -> Self {
        Self {
            incoming: state_root.join("queue/incoming"),
            processing: state_root.join("queue/processing"),
            outgoing: state_root.join("queue/outgoing"),
        }
    }
}

pub fn outgoing_filename(channel: &str, message_id: &str, timestamp: i64) -> String {
    if channel == "heartbeat" {
        format!("{}.json", sanitize_filename_component(message_id))
    } else {
        format!(
            "{}_{}_{}.json",
            sanitize_filename_component(channel),
            sanitize_filename_component(message_id),
            timestamp
        )
    }
}

pub fn is_valid_queue_json_filename(filename: &str) -> bool {
    let path = Path::new(filename);
    if path.extension().and_then(|v| v.to_str()) != Some("json") {
        return false;
    }

    if let Some(stem) = path.file_stem().and_then(|v| v.to_str()) {
        return !stem.trim().is_empty();
    }

    false
}

fn sanitize_filename_component(raw: &str) -> String {
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
