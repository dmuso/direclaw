use crate::queue::{IncomingMessage, OutgoingMessage, QueuePaths};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub fn recover_processing_queue_entries(state_root: &Path) -> Result<Vec<PathBuf>, String> {
    let queue_paths = QueuePaths::from_state_root(state_root);
    Ok(recover_queue_processing_paths(&queue_paths)?.recovered)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ProcessingRecoveryReport {
    pub(crate) recovered: Vec<PathBuf>,
    pub(crate) dropped_duplicates: Vec<PathBuf>,
}

pub(crate) fn recover_queue_processing_paths(
    queue_paths: &QueuePaths,
) -> Result<ProcessingRecoveryReport, String> {
    let mut recovered = Vec::new();
    let mut dropped_duplicates = Vec::new();
    let mut seen_processing_keys = BTreeSet::<(String, String)>::new();
    let outgoing_keys = collect_outgoing_message_keys(queue_paths)?;
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
        if let Some(key) = processing_message_key(&processing_path)? {
            if outgoing_keys.contains(&key) || !seen_processing_keys.insert(key) {
                fs::remove_file(&processing_path).map_err(|e| {
                    format!(
                        "failed to drop duplicate processing file {}: {}",
                        processing_path.display(),
                        e
                    )
                })?;
                dropped_duplicates.push(processing_path);
                continue;
            }
        }

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

    Ok(ProcessingRecoveryReport {
        recovered,
        dropped_duplicates,
    })
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

fn processing_message_key(processing_path: &Path) -> Result<Option<(String, String)>, String> {
    let raw = fs::read_to_string(processing_path).map_err(|e| {
        format!(
            "failed to read processing file {}: {}",
            processing_path.display(),
            e
        )
    })?;
    let incoming: IncomingMessage = match serde_json::from_str::<IncomingMessage>(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    Ok(Some((incoming.channel, incoming.message_id)))
}

fn collect_outgoing_message_keys(
    queue_paths: &QueuePaths,
) -> Result<BTreeSet<(String, String)>, String> {
    let mut keys = BTreeSet::<(String, String)>::new();
    let read_dir = match fs::read_dir(&queue_paths.outgoing) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(keys),
        Err(err) => return Err(err.to_string()),
    };
    for entry in read_dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let outgoing_raw = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read outgoing file {}: {}", path.display(), e))?;
        let outgoing: OutgoingMessage = match serde_json::from_str(&outgoing_raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        keys.insert((outgoing.channel, outgoing.message_id));
    }
    Ok(keys)
}
