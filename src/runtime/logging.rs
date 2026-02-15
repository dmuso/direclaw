use super::StatePaths;
use std::fs;
use std::io::Write;

pub fn append_runtime_log(paths: &StatePaths, level: &str, event: &str, message: &str) {
    let path = paths.runtime_log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let payload = serde_json::json!({
        "timestamp": super::now_secs(),
        "level": level,
        "event": event,
        "message": message,
    });

    let Ok(line) = serde_json::to_string(&payload) else {
        return;
    };

    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()));
}
