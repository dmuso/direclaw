use direclaw::config::OrchestratorConfig;
use direclaw::orchestrator::{
    enforce_execution_safety, evaluate_step_result, parse_and_validate_selector_result,
    resolve_selector_with_retries, route_selector_action, ExecutionSafetyLimits, FunctionRegistry,
    RoutedSelectorAction, RunState, SelectorAction, SelectorArtifactStore, SelectorRequest,
    SelectorResult, SelectorStatus, StatusResolutionInput, StepAttemptRecord, WorkflowRunStore,
};
use direclaw::queue::IncomingMessage;
use direclaw::runtime::{bootstrap_state_root, StatePaths};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use tempfile::tempdir;

fn sample_orchestrator() -> OrchestratorConfig {
    serde_yaml::from_str(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: engineering_default
selection_max_retries: 2
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.2
    can_orchestrate_workflows: false
workflows:
  - id: engineering_default
    version: 1
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
  - id: fix_issue
    version: 1
    steps:
      - id: plan
        type: agent_task
        agent: worker
        prompt: plan
      - id: review
        type: agent_review
        agent: worker
        prompt: review
        on_approve: done
        on_reject: plan
      - id: done
        type: agent_task
        agent: worker
        prompt: done
workflow_orchestration:
  max_total_iterations: 4
  default_run_timeout_seconds: 60
  default_step_timeout_seconds: 30
"#,
    )
    .expect("parse orchestrator")
}

fn sample_selector_request() -> SelectorRequest {
    SelectorRequest {
        selector_id: "selector-1".to_string(),
        channel_profile_id: "engineering".to_string(),
        message_id: "message-1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        user_message: "please fix this bug".to_string(),
        available_workflows: vec!["engineering_default".to_string(), "fix_issue".to_string()],
        default_workflow: "engineering_default".to_string(),
        available_functions: vec!["workflow.status".to_string(), "workflow.cancel".to_string()],
    }
}

fn sample_incoming() -> IncomingMessage {
    IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "Dana".to_string(),
        sender_id: "U100".to_string(),
        message: "please fix this bug".to_string(),
        timestamp: 1700000000,
        message_id: "message-1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    }
}

#[test]
fn selector_flow_persists_artifacts_and_supports_retry_and_default_fallback() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let orchestrator = sample_orchestrator();
    let request = sample_selector_request();
    let store = SelectorArtifactStore::new(&state_root);
    store
        .persist_message_snapshot(&sample_incoming())
        .expect("message snapshot");
    store
        .persist_selector_request(&request)
        .expect("request persisted");
    store
        .move_request_to_processing(&request.selector_id)
        .expect("request moved");

    let success = resolve_selector_with_retries(&orchestrator, &request, |attempt| {
        if attempt == 0 {
            Some("not-json".to_string())
        } else {
            Some(
                r#"{
              "selectorId":"selector-1",
              "status":"selected",
              "action":"workflow_start",
              "selectedWorkflow":"fix_issue"
            }"#
                .to_string(),
            )
        }
    });
    assert_eq!(success.retries_used, 1);
    assert!(!success.fell_back_to_default_workflow);
    assert_eq!(
        success.result.selected_workflow.as_deref(),
        Some("fix_issue")
    );
    store
        .persist_selector_result(&success.result)
        .expect("result persisted");
    store
        .persist_selector_log(&request.selector_id, "selector attempt logs")
        .expect("log persisted");

    assert!(state_root
        .join("orchestrator/messages/message-1.json")
        .is_file());
    assert!(state_root
        .join("orchestrator/select/processing/selector-1.json")
        .is_file());
    assert!(state_root
        .join("orchestrator/select/results/selector-1.json")
        .is_file());
    assert!(state_root
        .join("orchestrator/select/logs/selector-1.log")
        .is_file());

    let fallback =
        resolve_selector_with_retries(&orchestrator, &request, |_| Some("{}".to_string()));
    assert!(fallback.fell_back_to_default_workflow);
    assert_eq!(
        fallback.result.selected_workflow.as_deref(),
        Some("engineering_default")
    );
}

#[test]
fn selector_actions_route_status_without_advancing_and_restrict_commands_to_registry() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let store = WorkflowRunStore::new(&state_root);
    let mut run = store
        .create_run("run-1", "fix_issue", 10)
        .expect("create run");
    run.state = RunState::Running;
    run.current_step_id = Some("plan".to_string());
    run.current_attempt = Some(1);
    store.persist_run(&run).expect("persist run");
    let before = store.load_run("run-1").expect("load run before");

    let request = sample_selector_request();
    let status_result = SelectorResult {
        selector_id: "selector-1".to_string(),
        status: SelectorStatus::Selected,
        action: Some(SelectorAction::WorkflowStatus),
        selected_workflow: None,
        diagnostics_scope: None,
        function_id: None,
        function_args: None,
        reason: None,
    };
    let status_input = StatusResolutionInput {
        explicit_run_id: None,
        inbound_workflow_run_id: Some("run-1".to_string()),
        channel_profile_id: Some("engineering".to_string()),
        conversation_id: Some("thread-1".to_string()),
    };
    let registry = FunctionRegistry::new(vec!["workflow.status".to_string()]);
    let routed_status = route_selector_action(
        &request,
        &status_result,
        &status_input,
        &BTreeMap::new(),
        &registry,
    )
    .expect("status route");
    assert_eq!(
        routed_status,
        RoutedSelectorAction::WorkflowStatus {
            run_id: Some("run-1".to_string())
        }
    );

    let after = store.load_run("run-1").expect("load run after");
    assert_eq!(before.current_step_id, after.current_step_id);
    assert_eq!(before.current_attempt, after.current_attempt);

    let command_result = SelectorResult {
        selector_id: "selector-1".to_string(),
        status: SelectorStatus::Selected,
        action: Some(SelectorAction::CommandInvoke),
        selected_workflow: None,
        diagnostics_scope: None,
        function_id: Some("workflow.status".to_string()),
        function_args: Some(Map::from_iter([(
            "runId".to_string(),
            Value::String("run-1".to_string()),
        )])),
        reason: None,
    };
    let routed_command = route_selector_action(
        &request,
        &command_result,
        &status_input,
        &BTreeMap::new(),
        &registry,
    )
    .expect("command route");
    match routed_command {
        RoutedSelectorAction::CommandInvoke { result } => {
            assert_eq!(
                result["functionId"],
                Value::String("workflow.status".to_string())
            );
        }
        other => panic!("unexpected route: {other:?}"),
    }

    let unknown_function = r#"{
      "selectorId":"selector-1",
      "status":"selected",
      "action":"command_invoke",
      "functionId":"orchestrator.shutdown",
      "functionArgs":{}
    }"#;
    let err = parse_and_validate_selector_result(unknown_function, &request)
        .expect_err("reject unknown function");
    assert!(err.to_string().contains("availableFunctions"));
}

#[test]
fn run_state_and_progress_are_persisted_under_run_directory() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let store = WorkflowRunStore::new(&state_root);
    let mut run = store
        .create_run("run-2", "fix_issue", 100)
        .expect("create run");
    store
        .transition_state(
            &mut run,
            RunState::Running,
            101,
            "running step plan",
            false,
            "await step output",
        )
        .expect("running");

    let attempt = StepAttemptRecord {
        run_id: run.run_id.clone(),
        step_id: "plan".to_string(),
        attempt: 1,
        started_at: 102,
        ended_at: 103,
        state: "succeeded".to_string(),
        outputs: Map::from_iter([("plan".to_string(), Value::String("draft done".to_string()))]),
        next_step_id: Some("review".to_string()),
    };
    let attempt_path = store
        .persist_step_attempt(&attempt)
        .expect("step persisted");
    assert!(attempt_path.is_file());
    assert!(state_root.join("workflows/runs/run-2/run.json").is_file());
    assert!(state_root
        .join("workflows/runs/run-2/progress.json")
        .is_file());
    assert!(state_root
        .join("workflows/runs/run-2/steps/plan/attempt_1.json")
        .is_file());

    let progress = store.load_progress("run-2").expect("progress");
    assert_eq!(progress.state, RunState::Running);
    assert_eq!(progress.summary, "running step plan");

    let invalid = store.transition_state(&mut run, RunState::Queued, 104, "illegal", false, "none");
    assert!(invalid.is_err());
}

#[test]
fn step_execution_contract_enforces_envelope_routing_and_safety_controls() {
    let orchestrator = sample_orchestrator();
    let workflow = orchestrator
        .workflows
        .iter()
        .find(|w| w.id == "fix_issue")
        .expect("workflow");
    let plan = workflow
        .steps
        .iter()
        .find(|s| s.id == "plan")
        .expect("plan");
    let review = workflow
        .steps
        .iter()
        .find(|s| s.id == "review")
        .expect("review");

    let plan_out = evaluate_step_result(
        workflow,
        plan,
        r#"[workflow_result]{"artifact":"plan.md"}[/workflow_result]"#,
    )
    .expect("plan result");
    assert_eq!(plan_out.next_step_id.as_deref(), Some("review"));

    let reject_out = evaluate_step_result(
        workflow,
        review,
        r#"[workflow_result]{"decision":"reject","feedback":"missing tests"}[/workflow_result]"#,
    )
    .expect("review reject");
    assert_eq!(reject_out.next_step_id.as_deref(), Some("plan"));

    let approve_out = evaluate_step_result(
        workflow,
        review,
        r#"[workflow_result]{"decision":"approve","feedback":"ok"}[/workflow_result]"#,
    )
    .expect("review approve");
    assert_eq!(approve_out.next_step_id.as_deref(), Some("done"));

    let mut run = WorkflowRunStore::new(tempdir().expect("tempdir").path())
        .create_run("run-3", "fix_issue", 0)
        .expect("run");
    run.state = RunState::Running;
    run.total_iterations = 4;
    let limits = ExecutionSafetyLimits {
        max_total_iterations: 4,
        run_timeout_seconds: 20,
        step_timeout_seconds: 5,
        max_retries: 1,
    };
    assert!(enforce_execution_safety(&run, limits, 2, 0, 1).is_err());

    run.total_iterations = 0;
    assert!(enforce_execution_safety(&run, limits, 30, 0, 1).is_err());
    assert!(enforce_execution_safety(&run, limits, 7, 0, 1).is_err());
    assert!(enforce_execution_safety(&run, limits, 2, 0, 3).is_err());
}

#[test]
fn status_resolution_precedence_is_explicit_then_inbound_then_conversation() {
    let mut active = BTreeMap::new();
    active.insert(
        ("engineering".to_string(), "thread-1".to_string()),
        "run-conversation".to_string(),
    );

    let request = sample_selector_request();
    let status = SelectorResult {
        selector_id: request.selector_id.clone(),
        status: SelectorStatus::Selected,
        action: Some(SelectorAction::WorkflowStatus),
        selected_workflow: None,
        diagnostics_scope: None,
        function_id: None,
        function_args: None,
        reason: None,
    };
    let functions = FunctionRegistry::new(Vec::<String>::new());

    let explicit = route_selector_action(
        &request,
        &status,
        &StatusResolutionInput {
            explicit_run_id: Some("run-explicit".to_string()),
            inbound_workflow_run_id: Some("run-inbound".to_string()),
            channel_profile_id: Some("engineering".to_string()),
            conversation_id: Some("thread-1".to_string()),
        },
        &active,
        &functions,
    )
    .expect("explicit");
    assert_eq!(
        explicit,
        RoutedSelectorAction::WorkflowStatus {
            run_id: Some("run-explicit".to_string())
        }
    );

    let inbound = route_selector_action(
        &request,
        &status,
        &StatusResolutionInput {
            explicit_run_id: None,
            inbound_workflow_run_id: Some("run-inbound".to_string()),
            channel_profile_id: Some("engineering".to_string()),
            conversation_id: Some("thread-1".to_string()),
        },
        &active,
        &functions,
    )
    .expect("inbound");
    assert_eq!(
        inbound,
        RoutedSelectorAction::WorkflowStatus {
            run_id: Some("run-inbound".to_string())
        }
    );

    let conversation = route_selector_action(
        &request,
        &status,
        &StatusResolutionInput {
            explicit_run_id: None,
            inbound_workflow_run_id: None,
            channel_profile_id: Some("engineering".to_string()),
            conversation_id: Some("thread-1".to_string()),
        },
        &active,
        &functions,
    )
    .expect("conversation");
    assert_eq!(
        conversation,
        RoutedSelectorAction::WorkflowStatus {
            run_id: Some("run-conversation".to_string())
        }
    );
}
