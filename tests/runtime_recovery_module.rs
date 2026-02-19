use direclaw::queue::QueuePaths;
use direclaw::queue::{IncomingMessage, OutgoingMessage};
use direclaw::runtime::recovery::recover_processing_queue_entries;
use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths};
use std::fs;
use tempfile::tempdir;

#[test]
fn runtime_recovery_module_requeues_processing_entries_to_incoming() {
    let tmp = tempdir().expect("tempdir");
    let state_root = tmp.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let queue = QueuePaths::from_state_root(&state_root);
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::write(queue.processing.join("task.json"), b"{}").expect("write processing file");

    let recovered = recover_processing_queue_entries(&state_root).expect("recover entries");

    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].starts_with(&queue.incoming));
    assert!(queue
        .processing
        .read_dir()
        .expect("read processing")
        .next()
        .is_none());
}

#[test]
fn runtime_recovery_handles_near_limit_processing_filenames() {
    let tmp = tempdir().expect("tempdir");
    let state_root = tmp.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let queue = QueuePaths::from_state_root(&state_root);
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    let long_name = format!("{}.json", "b".repeat(240));
    fs::write(queue.processing.join(long_name), b"{}").expect("write processing file");

    let recovered = recover_processing_queue_entries(&state_root).expect("recover entries");
    assert_eq!(recovered.len(), 1);
    let name = recovered[0]
        .file_name()
        .and_then(|v| v.to_str())
        .expect("name");
    assert!(
        name.len() < 255,
        "recovered name should stay bounded: {}",
        name.len()
    );
}

#[test]
fn runtime_recovery_drops_processing_entry_when_outgoing_for_same_message_exists() {
    let tmp = tempdir().expect("tempdir");
    let state_root = tmp.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let queue = QueuePaths::from_state_root(&state_root);
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let message_id = "slack-profile-main-C001-1700000000_1";
    let incoming = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("slack_main".to_string()),
        sender: "U123".to_string(),
        sender_id: "U123".to_string(),
        message: "hello".to_string(),
        timestamp: 1700000000,
        message_id: message_id.to_string(),
        conversation_id: Some("C001:1700000000.1".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    fs::write(
        queue.processing.join("stale-processing.json"),
        serde_json::to_vec_pretty(&incoming).expect("serialize processing payload"),
    )
    .expect("write processing payload");

    let outgoing = OutgoingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("slack_main".to_string()),
        sender: "assistant".to_string(),
        message: "already produced".to_string(),
        original_message: "hello".to_string(),
        timestamp: 1700000001,
        message_id: message_id.to_string(),
        agent: "orchestrator".to_string(),
        conversation_id: Some("C001:1700000000.1".to_string()),
        target_ref: None,
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    fs::write(
        queue
            .outgoing
            .join("slack_slack-profile-main-C001-1700000000_1_1700000001.json"),
        serde_json::to_vec_pretty(&outgoing).expect("serialize outgoing payload"),
    )
    .expect("write outgoing payload");

    let recovered = recover_processing_queue_entries(&state_root).expect("recover entries");
    assert!(
        recovered.is_empty(),
        "stale processing should be dropped when outgoing already exists"
    );
    assert!(queue
        .processing
        .read_dir()
        .expect("read processing")
        .next()
        .is_none());
    assert!(queue
        .incoming
        .read_dir()
        .expect("read incoming")
        .next()
        .is_none());
}

#[test]
fn runtime_recovery_deduplicates_same_message_within_processing() {
    let tmp = tempdir().expect("tempdir");
    let state_root = tmp.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let queue = QueuePaths::from_state_root(&state_root);
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let message_id = "slack-profile-main-C001-1700000000_1";
    let incoming = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("slack_main".to_string()),
        sender: "U123".to_string(),
        sender_id: "U123".to_string(),
        message: "hello".to_string(),
        timestamp: 1700000000,
        message_id: message_id.to_string(),
        conversation_id: Some("C001:1700000000.1".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    fs::write(
        queue.processing.join("stale-processing-1.json"),
        serde_json::to_vec_pretty(&incoming).expect("serialize processing payload"),
    )
    .expect("write processing payload");
    fs::write(
        queue.processing.join("stale-processing-2.json"),
        serde_json::to_vec_pretty(&incoming).expect("serialize duplicate processing payload"),
    )
    .expect("write duplicate processing payload");

    let recovered = recover_processing_queue_entries(&state_root).expect("recover entries");
    assert_eq!(
        recovered.len(),
        1,
        "only one processing entry should be recovered for a given message id"
    );
    assert!(queue
        .processing
        .read_dir()
        .expect("read processing")
        .next()
        .is_none());
}
