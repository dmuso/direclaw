use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn append_memory_event(
    path: &Path,
    event: &str,
    fields: &[(&str, Value)],
) -> Result<(), std::io::Error> {
    let mut payload = Map::new();
    payload.insert("timestamp".to_string(), Value::from(now_secs()));
    payload.insert("event".to_string(), Value::String(event.to_string()));
    for (key, value) in fields {
        payload.insert((*key).to_string(), value.clone());
    }

    let line = serde_json::to_string(&payload)
        .map_err(|source| std::io::Error::other(source.to_string()))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
