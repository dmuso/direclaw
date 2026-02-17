use super::StatePaths;
use std::fs;
use std::io::Write;

pub fn append_runtime_log(paths: &StatePaths, level: &str, event: &str, message: &str) {
    let payload = serde_json::json!({
        "timestamp": super::now_secs(),
        "level": level,
        "event": event,
        "message": message,
    });

    let Ok(line) = serde_json::to_string(&payload) else {
        return;
    };

    let path = paths.runtime_log_path();
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "{line}");
}
