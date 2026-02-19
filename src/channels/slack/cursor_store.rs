use super::SlackError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SlackCursorState {
    #[serde(default)]
    pub conversations: BTreeMap<String, String>,
    #[serde(default)]
    pub threads: BTreeMap<String, String>,
}

fn io_error(path: &Path, source: std::io::Error) -> SlackError {
    SlackError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn json_error(path: &Path, source: serde_json::Error) -> SlackError {
    SlackError::Json {
        path: path.display().to_string(),
        source,
    }
}

fn sanitize_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn cursor_state_path(state_root: &Path, profile_id: &str) -> PathBuf {
    state_root
        .join("channels/slack")
        .join(sanitize_component(profile_id))
        .join("cursor.json")
}

pub fn load_cursor_state(
    state_root: &Path,
    profile_id: &str,
) -> Result<SlackCursorState, SlackError> {
    let path = cursor_state_path(state_root, profile_id);
    if !path.exists() {
        return Ok(SlackCursorState::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
    serde_json::from_str(&raw).map_err(|e| json_error(&path, e))
}

pub fn save_cursor_state(
    state_root: &Path,
    profile_id: &str,
    state: &SlackCursorState,
) -> Result<(), SlackError> {
    let path = cursor_state_path(state_root, profile_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(state).map_err(|e| json_error(&path, e))?;
    fs::write(&tmp, body).map_err(|e| io_error(&tmp, e))?;
    fs::rename(&tmp, &path).map_err(|e| io_error(&path, e))
}
