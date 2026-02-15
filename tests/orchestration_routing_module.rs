use direclaw::orchestration::routing::{
    process_queued_message, resolve_status_run_id, FunctionRegistry, StatusResolutionInput,
};
use direclaw::queue::IncomingMessage;
use std::collections::BTreeMap;
use tempfile::tempdir;

#[test]
fn routing_module_exposes_status_resolution_and_function_catalog() {
    let input = StatusResolutionInput {
        explicit_run_id: None,
        inbound_workflow_run_id: Some("run-inbound".to_string()),
        channel_profile_id: Some("engineering".to_string()),
        conversation_id: Some("thread-1".to_string()),
    };
    let active = BTreeMap::from_iter([(
        ("engineering".to_string(), "thread-1".to_string()),
        "run-active".to_string(),
    )]);

    let resolved = resolve_status_run_id(&input, &active);
    assert_eq!(resolved.as_deref(), Some("run-inbound"));

    let ids = FunctionRegistry::new(vec![
        "workflow.status".to_string(),
        "orchestrator.list".to_string(),
    ])
    .available_function_ids();
    assert!(ids.iter().any(|id| id == "workflow.status"));
    assert!(ids.iter().any(|id| id == "orchestrator.list"));
}

#[test]
fn routing_module_process_queued_message_exposes_entrypoint() {
    let settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
"#,
    )
    .expect("settings");

    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: None,
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "status".to_string(),
        timestamp: 1,
        message_id: "m1".to_string(),
        conversation_id: Some("c1".to_string()),
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let state = tempdir().expect("tempdir");
    let err = process_queued_message(
        state.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::new(Vec::new()),
        |_attempt, _request, _orchestrator| None,
    )
    .expect_err("missing channel profile should fail");
    assert!(err.to_string().contains("missing `channelProfileId`"));
}
