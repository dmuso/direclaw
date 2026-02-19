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
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
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

#[test]
fn incoming_message_uses_is_mentioned_wire_field() {
    let raw = r#"{
  "channel":"slack",
  "channelProfileId":"profile-1",
  "sender":"alice",
  "senderId":"U123",
  "message":"hello",
  "timestamp":1,
  "messageId":"m1",
  "conversationId":"thread-1",
  "isDirect":false,
  "isThreadReply":false,
  "isMentioned":true
}"#;
    let incoming: IncomingMessage = serde_json::from_str(raw).expect("parse incoming");
    assert!(incoming.is_mentioned);
    let encoded = serde_json::to_value(&incoming).expect("encode incoming");
    assert_eq!(
        encoded.get("isMentioned").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(
        encoded.get("mentionsProfile").is_none(),
        "legacy wire key must not be emitted"
    );
}
