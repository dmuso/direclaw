use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::SystemTime;

pub mod file_tags;
pub use file_tags::{
    append_inbound_file_tags, extract_inbound_file_tags, prepare_outbound_content,
};

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("queue io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid queue payload in {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IncomingMessage {
    pub channel: String,
    #[serde(default)]
    pub channel_profile_id: Option<String>,
    pub sender: String,
    pub sender_id: String,
    pub message: String,
    pub timestamp: i64,
    pub message_id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub workflow_run_id: Option<String>,
    #[serde(default)]
    pub workflow_step_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingMessage {
    pub channel: String,
    #[serde(default)]
    pub channel_profile_id: Option<String>,
    pub sender: String,
    pub message: String,
    pub original_message: String,
    pub timestamp: i64,
    pub message_id: String,
    pub agent: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub workflow_run_id: Option<String>,
    #[serde(default)]
    pub workflow_step_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuePaths {
    pub incoming: PathBuf,
    pub processing: PathBuf,
    pub outgoing: PathBuf,
}

pub const OUTBOUND_MAX_CHARS: usize = 4000;
pub const OUTBOUND_TRUNCATE_KEEP_CHARS: usize = 3900;
pub const OUTBOUND_TRUNCATION_SUFFIX: &str = "\n\n[Response truncated...]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundContent {
    pub message: String,
    pub files: Vec<String>,
    pub omitted_files: Vec<String>,
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

impl QueuePaths {
    pub fn from_state_root(state_root: &Path) -> Self {
        Self {
            incoming: state_root.join("queue/incoming"),
            processing: state_root.join("queue/processing"),
            outgoing: state_root.join("queue/outgoing"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaimedMessage {
    pub incoming_path: PathBuf,
    pub processing_path: PathBuf,
    pub payload: IncomingMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OrderingKey {
    WorkflowRun(String),
    Conversation {
        channel: String,
        channel_profile_id: String,
        conversation_id: String,
    },
    Message(String),
}

pub fn derive_ordering_key(payload: &IncomingMessage) -> OrderingKey {
    if let Some(workflow_run_id) = payload
        .workflow_run_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return OrderingKey::WorkflowRun(workflow_run_id.clone());
    }

    if let (Some(channel_profile_id), Some(conversation_id)) = (
        payload
            .channel_profile_id
            .as_ref()
            .filter(|s| !s.trim().is_empty()),
        payload
            .conversation_id
            .as_ref()
            .filter(|s| !s.trim().is_empty()),
    ) {
        return OrderingKey::Conversation {
            channel: payload.channel.clone(),
            channel_profile_id: channel_profile_id.clone(),
            conversation_id: conversation_id.clone(),
        };
    }

    OrderingKey::Message(payload.message_id.clone())
}

#[derive(Debug)]
pub struct Scheduled<T> {
    pub key: OrderingKey,
    pub value: T,
}

#[derive(Debug)]
pub struct PerKeyScheduler<T> {
    pending: VecDeque<Scheduled<T>>,
    active_keys: HashSet<OrderingKey>,
}

impl<T> Default for PerKeyScheduler<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
            active_keys: HashSet::new(),
        }
    }
}

impl<T> PerKeyScheduler<T> {
    pub fn enqueue(&mut self, key: OrderingKey, value: T) {
        self.pending.push_back(Scheduled { key, value });
    }

    pub fn dequeue_runnable(&mut self, max_items: usize) -> Vec<Scheduled<T>> {
        if max_items == 0 || self.pending.is_empty() {
            return Vec::new();
        }

        let mut selected = Vec::new();
        let mut selected_keys = HashSet::new();
        let mut remaining = VecDeque::new();

        while let Some(item) = self.pending.pop_front() {
            let key_busy =
                self.active_keys.contains(&item.key) || selected_keys.contains(&item.key);
            if !key_busy && selected.len() < max_items {
                selected_keys.insert(item.key.clone());
                self.active_keys.insert(item.key.clone());
                selected.push(item);
            } else {
                remaining.push_back(item);
            }
        }

        self.pending = remaining;
        selected
    }

    pub fn complete(&mut self, key: &OrderingKey) {
        self.active_keys.remove(key);
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn active_len(&self) -> usize {
        self.active_keys.len()
    }

    pub fn drain_pending(&mut self) -> Vec<Scheduled<T>> {
        self.pending.drain(..).collect()
    }
}

pub fn outgoing_filename(channel: &str, message_id: &str, timestamp: i64) -> String {
    if channel == "heartbeat" {
        format!("{}.json", sanitize_filename_component(message_id))
    } else {
        format!(
            "{}_{}_{}.json",
            sanitize_filename_component(channel),
            sanitize_filename_component(message_id),
            timestamp
        )
    }
}

pub fn is_valid_queue_json_filename(filename: &str) -> bool {
    let path = Path::new(filename);
    if path.extension().and_then(|v| v.to_str()) != Some("json") {
        return false;
    }

    if let Some(stem) = path.file_stem().and_then(|v| v.to_str()) {
        return !stem.trim().is_empty();
    }

    false
}

fn sanitize_filename_component(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_incoming_file(dir: &Path, name: &str, payload: &IncomingMessage) {
        let path = dir.join(name);
        fs::write(
            path,
            serde_json::to_string(payload).expect("serialize payload"),
        )
        .expect("write incoming");
    }

    fn sample_incoming(message_id: &str) -> IncomingMessage {
        IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("profile-1".to_string()),
            sender: "Alice".to_string(),
            sender_id: "U123".to_string(),
            message: "hello".to_string(),
            timestamp: 1,
            message_id: message_id.to_string(),
            conversation_id: Some("thread-1".to_string()),
            files: vec![],
            workflow_run_id: None,
            workflow_step_id: None,
        }
    }

    #[test]
    fn outgoing_filename_rules_match_spec() {
        assert_eq!(outgoing_filename("heartbeat", "hb-1", 100), "hb-1.json");
        assert_eq!(outgoing_filename("slack", "m1", 100), "slack_m1_100.json");
    }

    #[test]
    fn queue_claims_oldest_file_first() {
        let tmp = tempdir().expect("tempdir");
        let queue = QueuePaths::from_state_root(tmp.path());
        fs::create_dir_all(&queue.incoming).expect("incoming dir");
        fs::create_dir_all(&queue.processing).expect("processing dir");
        fs::create_dir_all(&queue.outgoing).expect("outgoing dir");

        write_incoming_file(&queue.incoming, "a.json", &sample_incoming("a"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        write_incoming_file(&queue.incoming, "b.json", &sample_incoming("b"));

        let claim = claim_oldest(&queue).expect("claim").expect("a claim");
        assert_eq!(claim.payload.message_id, "a");
        assert!(claim.processing_path.exists());
        assert!(!claim.incoming_path.exists());
    }

    #[test]
    fn requeue_moves_processing_back_to_incoming() {
        let tmp = tempdir().expect("tempdir");
        let queue = QueuePaths::from_state_root(tmp.path());
        fs::create_dir_all(&queue.incoming).expect("incoming dir");
        fs::create_dir_all(&queue.processing).expect("processing dir");
        fs::create_dir_all(&queue.outgoing).expect("outgoing dir");
        write_incoming_file(&queue.incoming, "a.json", &sample_incoming("a"));

        let claim = claim_oldest(&queue).expect("claim").expect("item");
        let requeued = requeue_failure(&queue, &claim).expect("requeue");
        assert!(requeued.exists());
        assert!(!claim.processing_path.exists());
    }

    #[test]
    fn derive_ordering_key_prefers_workflow_then_conversation() {
        let mut payload = sample_incoming("m1");
        payload.workflow_run_id = Some("run-1".to_string());
        assert_eq!(
            derive_ordering_key(&payload),
            OrderingKey::WorkflowRun("run-1".to_string())
        );

        payload.workflow_run_id = None;
        assert_eq!(
            derive_ordering_key(&payload),
            OrderingKey::Conversation {
                channel: "slack".to_string(),
                channel_profile_id: "profile-1".to_string(),
                conversation_id: "thread-1".to_string(),
            }
        );

        payload.channel_profile_id = None;
        assert_eq!(
            derive_ordering_key(&payload),
            OrderingKey::Message("m1".to_string())
        );
    }

    #[test]
    fn scheduler_allows_independent_keys_without_reordering_same_key() {
        let key_a = OrderingKey::WorkflowRun("run-a".to_string());
        let key_b = OrderingKey::WorkflowRun("run-b".to_string());

        let mut scheduler = PerKeyScheduler::default();
        scheduler.enqueue(key_a.clone(), "a1");
        scheduler.enqueue(key_a.clone(), "a2");
        scheduler.enqueue(key_b.clone(), "b1");

        let batch = scheduler.dequeue_runnable(2);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].value, "a1");
        assert_eq!(batch[1].value, "b1");

        scheduler.complete(&key_b);
        let blocked = scheduler.dequeue_runnable(2);
        assert!(blocked.is_empty());

        scheduler.complete(&key_a);
        let next = scheduler.dequeue_runnable(1);
        assert_eq!(next[0].value, "a2");
    }

    #[test]
    fn inbound_file_tag_extraction_and_append_are_deterministic() {
        let text = "hello [file: /tmp/a.txt] and [file: relative.txt] [file: /tmp/b.txt]";
        let tags = extract_inbound_file_tags(text);
        assert_eq!(tags, vec!["/tmp/a.txt", "/tmp/b.txt"]);

        let rendered = append_inbound_file_tags(
            "base",
            &[
                "/tmp/one.png".to_string(),
                "relative.png".to_string(),
                "/tmp/two.png".to_string(),
            ],
        );
        assert_eq!(rendered, "base\n[file: /tmp/one.png]\n[file: /tmp/two.png]");
    }

    #[test]
    fn outbound_send_file_tags_are_stripped_and_truncated_after_strip() {
        let tmp = tempdir().expect("tempdir");
        let sendable = tmp.path().join("artifact.txt");
        fs::write(&sendable, "x").expect("write file");

        let raw = format!(
            "preface [send_file: {}] tail [send_file: relative.txt]",
            sendable.display()
        );
        let prepared = prepare_outbound_content(&raw);
        assert_eq!(prepared.files, vec![sendable.display().to_string()]);
        assert_eq!(prepared.omitted_files, vec!["relative.txt".to_string()]);
        assert!(!prepared.message.contains("[send_file:"));

        let long = "a".repeat(4100);
        let prepared_long = prepare_outbound_content(&long);
        assert_eq!(prepared_long.message.chars().count(), 3925);
        assert!(prepared_long
            .message
            .ends_with("\n\n[Response truncated...]"));
    }
}
