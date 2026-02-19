use direclaw::queue::scheduler::{derive_ordering_key, PerKeyScheduler};
use direclaw::queue::{IncomingMessage, OrderingKey};

#[test]
fn queue_scheduler_module_exposes_existing_scheduler_apis() {
    let payload = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("eng".to_string()),
        sender: "Dana".to_string(),
        sender_id: "U42".to_string(),
        message: "hello".to_string(),
        timestamp: 100,
        message_id: "m-1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: Some("run-1".to_string()),
        workflow_step_id: None,
    };

    assert_eq!(
        derive_ordering_key(&payload),
        OrderingKey::WorkflowRun("run-1".to_string())
    );

    let mut scheduler = PerKeyScheduler::default();
    scheduler.enqueue(OrderingKey::WorkflowRun("run-a".to_string()), "a1");
    assert_eq!(scheduler.dequeue_runnable(1).len(), 1);
}
