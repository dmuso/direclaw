use direclaw::config::OrchestratorConfig;
use direclaw::orchestration::function_registry::FunctionRegistry;
use direclaw::orchestration::lexical_router::{
    resolve_lexical_decision, LexicalRoutingConfig, LexicalSelectorSource,
};
use direclaw::orchestration::selector::{SelectorAction, SelectorRequest};

fn sample_orchestrator() -> OrchestratorConfig {
    serde_yaml::from_str(
        r#"
id: engineering
selector_agent: router
default_workflow: quick_answer
selection_max_retries: 1
agents:
  router:
    provider: openai
    model: gpt-5.2
    can_orchestrate_workflows: true
workflows:
  - id: quick_answer
    version: 1
    description: General fallback workflow for broad requests
    tags: [general, default]
    steps:
      - id: step_1
        type: agent_task
        agent: router
        prompt: quick answer
        outputs: [summary]
        output_files:
          summary: summary.txt
  - id: deploy_release
    version: 1
    description: Handle deployment and release rollout operations
    tags: [deploy, release]
    steps:
      - id: step_1
        type: agent_task
        agent: router
        prompt: deploy request
        outputs: [summary]
        output_files:
          summary: summary.txt
"#,
    )
    .expect("orchestrator")
}

fn request(user_message: &str) -> SelectorRequest {
    SelectorRequest {
        selector_id: "sel-1".to_string(),
        channel_profile_id: "engineering".to_string(),
        message_id: "msg-1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        user_message: user_message.to_string(),
        memory_bulletin: None,
        memory_bulletin_citations: Vec::new(),
        available_workflows: vec!["quick_answer".to_string(), "deploy_release".to_string()],
        default_workflow: "quick_answer".to_string(),
        available_functions: vec!["workflow.status".to_string()],
        available_function_schemas: FunctionRegistry::new(vec!["workflow.status".to_string()])
            .available_function_schemas(),
    }
}

#[test]
fn lexical_router_module_prefers_matching_workflow_and_status_intent() {
    let orchestrator = sample_orchestrator();
    let functions = FunctionRegistry::new(vec!["workflow.status".to_string()]);

    let start = resolve_lexical_decision(
        &request("please deploy release 42"),
        &orchestrator,
        &functions,
        &LexicalRoutingConfig::balanced(),
    )
    .expect("decision");
    assert_eq!(start.result.action, Some(SelectorAction::WorkflowStart));
    assert_eq!(
        start.result.selected_workflow.as_deref(),
        Some("deploy_release")
    );
    assert_eq!(start.source, LexicalSelectorSource::Lexical);

    let status = resolve_lexical_decision(
        &request("what is the latest status update?"),
        &orchestrator,
        &functions,
        &LexicalRoutingConfig::balanced(),
    )
    .expect("status decision");
    assert_eq!(status.result.action, Some(SelectorAction::WorkflowStatus));
}

#[test]
fn lexical_router_module_respects_negation_and_falls_back_on_low_confidence() {
    let orchestrator = sample_orchestrator();
    let functions = FunctionRegistry::new(vec!["workflow.status".to_string()]);
    let decision = resolve_lexical_decision(
        &request("don't deploy anything, use the default workflow"),
        &orchestrator,
        &functions,
        &LexicalRoutingConfig::balanced(),
    )
    .expect("decision");
    assert_eq!(
        decision.result.selected_workflow.as_deref(),
        Some("quick_answer")
    );

    let ambiguous = resolve_lexical_decision(
        &request("help"),
        &orchestrator,
        &functions,
        &LexicalRoutingConfig::high_precision(),
    );
    assert!(ambiguous.is_none());
}
