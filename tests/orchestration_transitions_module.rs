use direclaw::config::OrchestratorConfig;
use direclaw::orchestration::function_registry::FunctionRegistry;
use direclaw::orchestration::routing::StatusResolutionInput;
use direclaw::orchestration::run_store::WorkflowRunStore;
use direclaw::orchestration::selector::{
    SelectorAction, SelectorRequest, SelectorResult, SelectorStatus,
};
use direclaw::orchestration::transitions::{
    route_selector_action, RouteContext, RoutedSelectorAction,
};
use std::collections::BTreeMap;
use tempfile::tempdir;

fn sample_orchestrator() -> OrchestratorConfig {
    serde_yaml::from_str(
        r#"
id: eng
selector_agent: selector
default_workflow: default
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  selector:
    provider: openai
    model: gpt-4.1
workflows: []
"#,
    )
    .expect("orchestrator")
}

#[test]
fn transitions_module_routes_status_without_active_run() {
    let dir = tempdir().expect("tempdir");
    let store = WorkflowRunStore::new(dir.path());
    let request = SelectorRequest {
        selector_id: "sel-1".to_string(),
        channel_profile_id: "engineering".to_string(),
        message_id: "m-1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        user_message: "status".to_string(),
        thread_context: None,
        memory_bulletin: None,
        memory_bulletin_citations: Vec::new(),
        available_workflows: Vec::new(),
        default_workflow: "default".to_string(),
        available_functions: Vec::new(),
        available_function_schemas: Vec::new(),
    };
    let result = SelectorResult {
        selector_id: "sel-1".to_string(),
        status: SelectorStatus::Selected,
        action: Some(SelectorAction::WorkflowStatus),
        selected_workflow: None,
        diagnostics_scope: None,
        function_id: None,
        function_args: None,
        reason: None,
    };

    let routed = route_selector_action(
        &request,
        &result,
        RouteContext {
            status_input: &StatusResolutionInput {
                explicit_run_id: None,
                inbound_workflow_run_id: None,
                channel_profile_id: Some("engineering".to_string()),
                conversation_id: Some("thread-1".to_string()),
            },
            active_conversation_runs: &BTreeMap::new(),
            functions: &FunctionRegistry::new(Vec::<String>::new()),
            run_store: &store,
            orchestrator: &sample_orchestrator(),
            workspace_access_context: None,
            runner_binaries: None,
            memory_enabled: false,
            source_message_id: None,
            workflow_inputs: None,
            now: 100,
        },
    )
    .expect("route");

    match routed {
        RoutedSelectorAction::WorkflowStatus {
            run_id: None,
            progress: None,
            message,
        } => assert_eq!(
            message,
            "no active workflow run found for this conversation"
        ),
        other => panic!("unexpected route: {other:?}"),
    }
}
