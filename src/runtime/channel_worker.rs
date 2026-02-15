use crate::config::Settings;
use crate::slack;
use std::path::Path;

pub fn tick_slack_worker(state_root: &Path, settings: &Settings) -> Result<(), String> {
    slack::sync_once(state_root, settings)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
