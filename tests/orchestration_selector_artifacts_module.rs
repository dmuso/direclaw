use direclaw::orchestration::selector::SelectorRequest;
use direclaw::orchestration::selector_artifacts::SelectorArtifactStore;
use direclaw::queue::IncomingMessage;
use serde_json::Value;

fn sample_request() -> SelectorRequest {
    SelectorRequest {
        selector_id: "sel-1".to_string(),
        channel_profile_id: "engineering".to_string(),
        message_id: "m1".to_string(),
        conversation_id: Some("c1".to_string()),
        user_message: "status".to_string(),
        memory_bulletin: None,
        memory_bulletin_citations: Vec::new(),
        available_workflows: vec!["default".to_string()],
        default_workflow: "default".to_string(),
        available_functions: vec!["workflow.status".to_string()],
        available_function_schemas: Vec::new(),
    }
}

fn sample_incoming() -> IncomingMessage {
    IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "Dana".to_string(),
        sender_id: "U100".to_string(),
        message: "status".to_string(),
        timestamp: 1700000000,
        message_id: "m1".to_string(),
        conversation_id: Some("c1".to_string()),
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    }
}

#[test]
fn selector_artifact_store_module_path_persists_expected_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");

    let store = SelectorArtifactStore::new(&state_root);
    let request = sample_request();

    store
        .persist_message_snapshot(&sample_incoming())
        .expect("persist message");
    store
        .persist_selector_request(&request)
        .expect("persist request");
    store
        .move_request_to_processing(&request.selector_id)
        .expect("move request");

    let message: Value = serde_json::from_str(
        &std::fs::read_to_string(state_root.join("orchestrator/artifacts/message-m1.json"))
            .expect("read message"),
    )
    .expect("parse message json");
    assert_eq!(message["messageId"], Value::String("m1".to_string()));

    assert!(state_root
        .join("orchestrator/artifacts/selector-processing-sel-1.json")
        .is_file());
}
