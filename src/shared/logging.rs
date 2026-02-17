use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn orchestrator_log_path(state_root: &Path) -> PathBuf {
    state_root.join("logs/orchestrator.log")
}

pub fn append_orchestrator_log_line(state_root: &Path, line: &str) -> std::io::Result<()> {
    let path = orchestrator_log_path(state_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{line}")
}
