use crate::queue::QueuePaths;
use crate::shared::logging::append_orchestrator_log_line;
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
    let _ = append_orchestrator_log_line(&root, line);
}
