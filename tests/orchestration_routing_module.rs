use direclaw::orchestration::routing::{
    resolve_status_run_id, FunctionRegistry, StatusResolutionInput,
};
use std::collections::BTreeMap;

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
