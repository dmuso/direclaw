use crate::config::Settings;
use crate::memory::{bootstrap_memory_paths, process_ingest_once, MemoryPaths, MemoryRepository};
use serde_json::Value;
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
        MemoryRepository::open(&paths.database, orchestrator_id)
            .and_then(|repo| repo.ensure_schema())
            .map_err(|e| classify_memory_store_error(&paths.database, &e.to_string()))?;
        append_memory_log(
            &paths.log_file,
            "memory.worker.bootstrap_complete",
            &[(
                "orchestrator_id",
                Value::String(orchestrator_id.to_string()),
            )],
        )?;
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
        process_ingest_once(
            &paths,
            orchestrator_id,
            settings.memory.ingest.max_file_size_mb,
        )
        .map_err(|e| classify_memory_store_error(&paths.database, &e.to_string()))?;
        append_memory_log(
            &paths.log_file,
            "memory.worker.heartbeat",
            &[(
                "orchestrator_id",
                Value::String(orchestrator_id.to_string()),
            )],
        )?;
    }
    Ok(())
}

fn classify_memory_store_error(database_path: &Path, message: &str) -> String {
    if is_sqlite_corruption_message(message) {
        let normalized = format!(
            "memory_db_corrupt path={} detail={message}",
            database_path.display()
        );
        let log_path = database_path
            .parent()
            .map(|root| root.join("logs/memory.log"))
            .unwrap_or_else(|| database_path.to_path_buf());
        let _ = append_memory_log(
            &log_path,
            "memory.worker.degraded",
            &[
                (
                    "reason_code",
                    Value::String("memory_db_corrupt".to_string()),
                ),
                (
                    "database_path",
                    Value::String(database_path.display().to_string()),
                ),
                ("error", Value::String(message.to_string())),
            ],
        );
        normalized
    } else {
        message.to_string()
    }
}

fn is_sqlite_corruption_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("not a database")
        || normalized.contains("database disk image is malformed")
        || normalized.contains("file is not a database")
        || normalized.contains("sqlite format")
}

fn append_memory_log(path: &Path, event: &str, fields: &[(&str, Value)]) -> Result<(), String> {
    let mut payload = serde_json::Map::new();
    payload.insert("timestamp".to_string(), Value::from(now_secs()));
    payload.insert("event".to_string(), Value::String(event.to_string()));
    for (key, value) in fields {
        payload.insert((*key).to_string(), value.clone());
    }
    let line = serde_json::to_string(&payload).map_err(|source| source.to_string())?;

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

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
