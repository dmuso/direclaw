use direclaw::queue::message::{IncomingMessage, OutgoingMessage};

#[test]
fn queue_message_module_exposes_queue_message_types() {
    let incoming = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("profile-1".to_string()),
        sender: "alice".to_string(),
        sender_id: "U123".to_string(),
        message: "hello".to_string(),
        timestamp: 1,
        message_id: "m1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        files: vec!["/tmp/a.txt".to_string()],
        workflow_run_id: Some("run-1".to_string()),
        workflow_step_id: Some("step-1".to_string()),
    };

    let outgoing = OutgoingMessage {
        channel: incoming.channel.clone(),
        channel_profile_id: incoming.channel_profile_id.clone(),
        sender: "direclaw".to_string(),
        message: "done".to_string(),
        original_message: incoming.message.clone(),
        timestamp: incoming.timestamp,
        message_id: incoming.message_id.clone(),
        agent: "agent-1".to_string(),
        conversation_id: incoming.conversation_id.clone(),
        target_ref: None,
        files: incoming.files.clone(),
        workflow_run_id: incoming.workflow_run_id.clone(),
        workflow_step_id: incoming.workflow_step_id.clone(),
    };

    assert_eq!(outgoing.channel, "slack");
    assert_eq!(outgoing.files, vec!["/tmp/a.txt".to_string()]);
}
