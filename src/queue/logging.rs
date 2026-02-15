use crate::queue::QueuePaths;
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn append_queue_log(paths: &QueuePaths, line: &str) {
    let root = paths
        .incoming
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let Some(root) = root else {
        return;
    };
    let path = root.join("logs/security.log");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()));
}
