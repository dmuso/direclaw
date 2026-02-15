use super::{
    file_tags, is_valid_queue_json_filename, outgoing_filename, IncomingMessage, OutgoingMessage,
    QueueError, QueuePaths,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct ClaimedMessage {
    pub incoming_path: PathBuf,
    pub processing_path: PathBuf,
    pub payload: IncomingMessage,
}

pub fn claim_oldest(paths: &QueuePaths) -> Result<Option<ClaimedMessage>, QueueError> {
    for incoming_path in sorted_incoming_paths(&paths.incoming)? {
        let Some(file_name) = incoming_path.file_name() else {
            continue;
        };
        let processing_path = paths.processing.join(file_name);

        match fs::rename(&incoming_path, &processing_path) {
            Ok(_) => {
                let raw = match fs::read_to_string(&processing_path) {
                    Ok(raw) => raw,
                    Err(err) => {
                        requeue_processing_file(paths, &processing_path)?;
                        return Err(io_err(&processing_path, err));
                    }
                };
                let mut payload: IncomingMessage = match serde_json::from_str(&raw) {
                    Ok(payload) => payload,
                    Err(err) => {
                        requeue_processing_file(paths, &processing_path)?;
                        return Err(parse_err(&processing_path, err));
                    }
                };
                file_tags::normalize_inbound_payload(&mut payload);
                return Ok(Some(ClaimedMessage {
                    incoming_path,
                    processing_path,
                    payload,
                }));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(io_err(&incoming_path, err)),
        }
    }

    Ok(None)
}

pub fn complete_success(
    paths: &QueuePaths,
    claimed: &ClaimedMessage,
    outgoing: &OutgoingMessage,
) -> Result<PathBuf, QueueError> {
    let (normalized_outgoing, omitted_files) = file_tags::normalize_outgoing_message(outgoing);
    if !omitted_files.is_empty() {
        append_queue_log(
            paths,
            &format!(
                "outgoing message `{}` omitted invalid/unreadable files: {}",
                outgoing.message_id,
                omitted_files.join(", ")
            ),
        );
    }
    let filename = outgoing_filename(&outgoing.channel, &outgoing.message_id, outgoing.timestamp);
    let out_path = paths.outgoing.join(filename);
    let body =
        serde_json::to_string_pretty(&normalized_outgoing).map_err(|e| parse_err(&out_path, e))?;

    fs::write(&out_path, body).map_err(|e| io_err(&out_path, e))?;
    fs::remove_file(&claimed.processing_path).map_err(|e| io_err(&claimed.processing_path, e))?;
    Ok(out_path)
}

pub fn requeue_failure(
    paths: &QueuePaths,
    claimed: &ClaimedMessage,
) -> Result<PathBuf, QueueError> {
    requeue_processing_file(paths, &claimed.processing_path)
}

fn append_queue_log(paths: &QueuePaths, line: &str) {
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

fn io_err(path: &Path, source: std::io::Error) -> QueueError {
    QueueError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn parse_err(path: &Path, source: serde_json::Error) -> QueueError {
    QueueError::Parse {
        path: path.display().to_string(),
        source,
    }
}

fn sorted_incoming_paths(incoming_dir: &Path) -> Result<Vec<PathBuf>, QueueError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(incoming_dir).map_err(|e| io_err(incoming_dir, e))? {
        let entry = entry.map_err(|e| io_err(incoming_dir, e))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if !is_valid_queue_json_filename(name) {
                continue;
            }
        }

        let metadata = entry.metadata().map_err(|e| io_err(&path, e))?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        entries.push((modified, path));
    }

    entries.sort_by(|(a_time, a_path), (b_time, b_path)| {
        a_time
            .cmp(b_time)
            .then_with(|| a_path.file_name().cmp(&b_path.file_name()))
    });

    Ok(entries.into_iter().map(|(_, path)| path).collect())
}

static REQUEUE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_requeue_name(original_name: &str) -> String {
    let path = Path::new(original_name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("message");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("json");
    let counter = REQUEUE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    format!("{stem}_requeue_{counter}.{ext}")
}

fn requeue_processing_file(
    paths: &QueuePaths,
    processing_path: &Path,
) -> Result<PathBuf, QueueError> {
    let file_name = processing_path.file_name().ok_or_else(|| {
        io_err(
            processing_path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "processing file missing name",
            ),
        )
    })?;
    let file_name = file_name.to_string_lossy();
    let incoming = paths.incoming.join(unique_requeue_name(&file_name));
    fs::rename(processing_path, &incoming).map_err(|e| io_err(processing_path, e))?;
    Ok(incoming)
}
