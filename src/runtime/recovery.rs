use crate::queue::QueuePaths;
use std::fs;
use std::path::{Path, PathBuf};

pub fn recover_processing_queue_entries(state_root: &Path) -> Result<Vec<PathBuf>, String> {
    let queue_paths = QueuePaths::from_state_root(state_root);
    let mut recovered = Vec::new();
    let mut entries = Vec::new();

    for entry in fs::read_dir(&queue_paths.processing).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() {
            entries.push(path);
        }
    }
    entries.sort();

    for (index, processing_path) in entries.into_iter().enumerate() {
        let name = processing_path
            .file_name()
            .and_then(|v| v.to_str())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("message.json");
        let target = queue_paths
            .incoming
            .join(format!("recovered_{index}_{name}"));
        fs::rename(&processing_path, &target).map_err(|e| {
            format!(
                "failed to recover processing file {}: {}",
                processing_path.display(),
                e
            )
        })?;
        recovered.push(target);
    }

    Ok(recovered)
}
