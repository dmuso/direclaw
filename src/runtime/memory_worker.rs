use crate::config::Settings;
use crate::memory::{bootstrap_memory_paths, MemoryPaths};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub fn bootstrap_memory_runtime_paths(settings: &Settings) -> Result<(), String> {
    for orchestrator_id in settings.orchestrators.keys() {
        let runtime_root = settings
            .resolve_orchestrator_runtime_root(orchestrator_id)
            .map_err(|e| e.to_string())?;
        let paths = MemoryPaths::from_runtime_root(&runtime_root);
        bootstrap_memory_paths(&paths).map_err(|e| e.to_string())?;
        append_memory_log(&paths.log_file, "memory worker bootstrap complete")?;
    }
    Ok(())
}

pub fn tick_memory_worker(settings: &Settings) -> Result<(), String> {
    if !settings.memory.ingest.enabled {
        return Ok(());
    }

    for orchestrator_id in settings.orchestrators.keys() {
        let runtime_root = settings
            .resolve_orchestrator_runtime_root(orchestrator_id)
            .map_err(|e| e.to_string())?;
        let paths = MemoryPaths::from_runtime_root(&runtime_root);
        append_memory_log(&paths.log_file, "memory worker heartbeat")?;
    }
    Ok(())
}

fn append_memory_log(path: &Path, line: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create memory path {}: {e}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("failed to create memory path {}: {e}", path.display()))?;
    writeln!(file, "{line}").map_err(|e| format!("failed to write {}: {e}", path.display()))
}
