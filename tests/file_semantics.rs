use direclaw::queue::{
    append_inbound_file_tags, claim_oldest, complete_success, prepare_outbound_content,
    IncomingMessage, OutgoingMessage, QueuePaths,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn file_tags_round_trip_from_inbound_queue_to_outbound_payload() {
    let dir = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(dir.path());
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let inbound_file = dir.path().join("files/inbound.txt");
    fs::create_dir_all(inbound_file.parent().expect("parent")).expect("files dir");
    fs::write(&inbound_file, "inbound").expect("write inbound file");

    let inbound_file_str = inbound_file.display().to_string();
    let incoming = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "Dana".to_string(),
        sender_id: "U42".to_string(),
        message: append_inbound_file_tags("please review", std::slice::from_ref(&inbound_file_str)),
        timestamp: 100,
        message_id: "msg-file-1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![inbound_file_str.clone()],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    fs::write(
        queue.incoming.join("msg-file-1.json"),
        serde_json::to_string(&incoming).expect("serialize incoming"),
    )
    .expect("write incoming");

    let claimed = claim_oldest(&queue)
        .expect("claim")
        .expect("claimed payload");
    assert!(claimed
        .payload
        .message
        .contains(&format!("[file: {}]", inbound_file.display())));
    assert_eq!(claimed.payload.files, vec![inbound_file_str]);

    let outbound_file = dir.path().join("files/outbound.txt");
    fs::write(&outbound_file, "outbound").expect("write outbound file");

    let prepared = prepare_outbound_content(&format!(
        "summary complete [send_file: {}]",
        outbound_file.display()
    ));
    let outgoing = OutgoingMessage {
        channel: claimed.payload.channel.clone(),
        channel_profile_id: claimed.payload.channel_profile_id.clone(),
        sender: claimed.payload.sender.clone(),
        message: prepared.message,
        original_message: claimed.payload.message.clone(),
        timestamp: 200,
        message_id: claimed.payload.message_id.clone(),
        agent: "worker".to_string(),
        conversation_id: claimed.payload.conversation_id.clone(),
        target_ref: None,
        files: prepared.files,
        workflow_run_id: claimed.payload.workflow_run_id.clone(),
        workflow_step_id: claimed.payload.workflow_step_id.clone(),
    };

    let out_path = complete_success(&queue, &claimed, &outgoing).expect("persist outgoing");
    let saved: OutgoingMessage =
        serde_json::from_str(&fs::read_to_string(out_path).expect("read outgoing"))
            .expect("parse outgoing");
    assert!(!saved.message.contains("[send_file:"));
    assert_eq!(saved.files, vec![outbound_file.display().to_string()]);
}

#[test]
fn inbound_files_are_normalized_to_absolute_and_reflected_in_message_tags() {
    let dir = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(dir.path());
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let abs = dir.path().join("files/inbound.txt");
    fs::create_dir_all(abs.parent().expect("parent")).expect("create parent");
    fs::write(&abs, "x").expect("write");

    let payload = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "Dana".to_string(),
        sender_id: "U42".to_string(),
        message: "check this".to_string(),
        timestamp: 100,
        message_id: "msg-file-2".to_string(),
        conversation_id: Some("thread-1".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec!["relative.txt".to_string(), abs.display().to_string()],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    fs::write(
        queue.incoming.join("msg-file-2.json"),
        serde_json::to_string(&payload).expect("serialize"),
    )
    .expect("write payload");

    let claimed = claim_oldest(&queue).expect("claim").expect("claimed");
    assert_eq!(claimed.payload.files, vec![abs.display().to_string()]);
    assert!(claimed
        .payload
        .message
        .contains(&format!("[file: {}]", abs.display())));
}
