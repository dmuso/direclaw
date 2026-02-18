use direclaw::queue::{
    claim_oldest, complete_success, complete_success_many, requeue_failure, IncomingMessage,
    OutgoingMessage, QueuePaths,
};
use std::fs;
use tempfile::tempdir;

fn make_incoming(message_id: &str) -> IncomingMessage {
    IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("eng".to_string()),
        sender: "Dana".to_string(),
        sender_id: "U42".to_string(),
        message: "hello".to_string(),
        timestamp: 100,
        message_id: message_id.to_string(),
        conversation_id: Some("thread-1".to_string()),
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    }
}

fn make_outgoing(incoming: &IncomingMessage) -> OutgoingMessage {
    OutgoingMessage {
        channel: incoming.channel.clone(),
        channel_profile_id: incoming.channel_profile_id.clone(),
        sender: incoming.sender.clone(),
        message: "done".to_string(),
        original_message: incoming.message.clone(),
        timestamp: 300,
        message_id: incoming.message_id.clone(),
        agent: "worker".to_string(),
        conversation_id: incoming.conversation_id.clone(),
        target_ref: None,
        files: vec![],
        workflow_run_id: incoming.workflow_run_id.clone(),
        workflow_step_id: incoming.workflow_step_id.clone(),
    }
}

#[test]
fn queue_lifecycle_moves_incoming_to_processing_to_outgoing() {
    let dir = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(dir.path());
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let incoming = make_incoming("msg-1");
    let path = queue.incoming.join("msg-1.json");
    fs::write(&path, serde_json::to_string(&incoming).expect("serialize")).expect("write");

    let claimed = claim_oldest(&queue).expect("claim").expect("item");
    assert!(claimed.processing_path.exists());
    assert!(!path.exists());

    let outgoing = make_outgoing(&claimed.payload);
    let out_path = complete_success(&queue, &claimed, &outgoing).expect("complete success");

    assert!(!claimed.processing_path.exists());
    assert!(out_path.exists());

    let saved: OutgoingMessage =
        serde_json::from_str(&fs::read_to_string(out_path).expect("read outgoing")).expect("parse");
    assert_eq!(saved.message, "done");
}

#[test]
fn queue_failure_requeues_payload_without_loss() {
    let dir = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(dir.path());
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let incoming = make_incoming("msg-2");
    let raw = serde_json::to_string(&incoming).expect("serialize");
    let in_path = queue.incoming.join("msg-2.json");
    fs::write(&in_path, &raw).expect("write");

    let claimed = claim_oldest(&queue).expect("claim").expect("item");
    let requeued = requeue_failure(&queue, &claimed).expect("requeue");
    assert!(requeued.exists());

    let persisted = fs::read_to_string(&requeued).expect("read requeued");
    assert_eq!(persisted, raw);
}

#[test]
fn queue_claim_parse_failure_requeues_back_to_incoming() {
    let dir = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(dir.path());
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let in_path = queue.incoming.join("bad.json");
    fs::write(&in_path, "{not-json}").expect("write invalid json");

    let err = claim_oldest(&queue).expect_err("claim should fail");
    let err_text = err.to_string();
    assert!(err_text.contains("invalid queue payload"));
    assert!(fs::read_dir(&queue.processing)
        .expect("read processing")
        .next()
        .is_none());

    let mut incoming_files: Vec<_> = fs::read_dir(&queue.incoming)
        .expect("read incoming")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(incoming_files.len(), 1);
    let requeued = incoming_files.pop().expect("requeued file");
    assert_eq!(
        fs::read_to_string(requeued).expect("read requeued"),
        "{not-json}"
    );
}

#[test]
fn queue_requeue_failure_does_not_clobber_existing_incoming_file() {
    let dir = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(dir.path());
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let incoming = make_incoming("msg-3");
    let raw = serde_json::to_string(&incoming).expect("serialize");
    let in_path = queue.incoming.join("msg-3.json");
    fs::write(&in_path, &raw).expect("write");

    let claimed = claim_oldest(&queue).expect("claim").expect("item");

    // Simulate another item arriving with the original file name before requeue.
    fs::write(&in_path, "{\"new\":\"payload\"}").expect("write competing incoming");

    let requeued = requeue_failure(&queue, &claimed).expect("requeue");
    assert_ne!(requeued, in_path);
    assert!(in_path.exists());
    assert_eq!(
        fs::read_to_string(&in_path).expect("read competing payload"),
        "{\"new\":\"payload\"}"
    );
    assert_eq!(fs::read_to_string(requeued).expect("read requeued"), raw);
}

#[test]
fn complete_success_enforces_send_file_stripping_truncation_and_file_validation() {
    let dir = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(dir.path());
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let incoming = make_incoming("msg-4");
    fs::write(
        queue.incoming.join("msg-4.json"),
        serde_json::to_string(&incoming).expect("serialize"),
    )
    .expect("write");
    let claimed = claim_oldest(&queue).expect("claim").expect("item");

    let sendable = dir.path().join("artifact.txt");
    fs::write(&sendable, "artifact").expect("write artifact");
    let long = "a".repeat(4050);

    let mut outgoing = make_outgoing(&claimed.payload);
    outgoing.message = format!(
        "{long}[send_file: {}][send_file: relative.txt]",
        sendable.display()
    );
    outgoing.files = vec![
        sendable.display().to_string(),
        "relative.txt".to_string(),
        sendable.display().to_string(),
    ];
    let out_path = complete_success(&queue, &claimed, &outgoing).expect("complete success");

    let saved: OutgoingMessage =
        serde_json::from_str(&fs::read_to_string(&out_path).expect("read")).expect("parse");
    assert!(!saved.message.contains("[send_file:"));
    assert!(saved.message.ends_with("\n\n[Response truncated...]"));
    assert_eq!(saved.files, vec![sendable.display().to_string()]);

    let log_path = dir.path().join("logs/orchestrator.log");
    let log = fs::read_to_string(log_path).expect("read security log");
    assert!(log.contains("omitted invalid/unreadable files"));
}

#[test]
fn complete_success_many_writes_ordered_outbound_messages_for_one_claim() {
    let dir = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(dir.path());
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let incoming = make_incoming("msg-5");
    fs::write(
        queue.incoming.join("msg-5.json"),
        serde_json::to_string(&incoming).expect("serialize"),
    )
    .expect("write");

    let claimed = claim_oldest(&queue).expect("claim").expect("item");
    let mut first = make_outgoing(&claimed.payload);
    first.message = "step one".to_string();
    let mut second = make_outgoing(&claimed.payload);
    second.message = "step two".to_string();
    second.timestamp = first.timestamp.saturating_add(1);

    let written = complete_success_many(&queue, &claimed, &[first.clone(), second.clone()])
        .expect("complete many");
    assert_eq!(written.len(), 2);
    assert!(!claimed.processing_path.exists());

    let mut files: Vec<_> = fs::read_dir(&queue.outgoing)
        .expect("outgoing")
        .map(|entry| entry.expect("entry").path())
        .collect();
    files.sort();
    assert_eq!(files.len(), 2);

    let parsed: Vec<OutgoingMessage> = files
        .iter()
        .map(|path| serde_json::from_str(&fs::read_to_string(path).expect("read")).expect("parse"))
        .collect();
    assert_eq!(parsed[0].message, "step one");
    assert_eq!(parsed[1].message, "step two");
}
