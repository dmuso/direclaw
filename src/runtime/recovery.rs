use crate::queue::QueuePaths;
use sha2::{Digest, Sha256};
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
            .join(recovered_processing_filename(index, name));
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

pub(crate) fn recovered_processing_filename(index: usize, name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("json");
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();
    let hash = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("recovered_{index}_{hash}.{ext}")
}
