use direclaw::queue::lifecycle::{claim_oldest, requeue_failure};
use direclaw::queue::{IncomingMessage, QueuePaths};
use std::fs;
use tempfile::tempdir;

#[test]
fn queue_lifecycle_module_exposes_existing_lifecycle_apis() {
    let tmp = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(tmp.path());
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
    fs::create_dir_all(&queue.processing).expect("processing dir");
    fs::create_dir_all(&queue.outgoing).expect("outgoing dir");

    let payload = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("eng".to_string()),
        sender: "Dana".to_string(),
        sender_id: "U42".to_string(),
        message: "hello".to_string(),
        timestamp: 100,
        message_id: "m-1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    fs::write(
        queue.incoming.join("a.json"),
        serde_json::to_string(&payload).expect("serialize payload"),
    )
    .expect("write incoming");

    let claimed = claim_oldest(&queue).expect("claim").expect("item");
    let requeued = requeue_failure(&queue, &claimed).expect("requeue");
    assert!(requeued.exists());
}
