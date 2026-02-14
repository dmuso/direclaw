use direclaw::config::{OrchestratorConfig, Settings};
use direclaw::orchestrator::{
    enforce_execution_safety, evaluate_step_result, parse_and_validate_selector_result,
    process_queued_message, resolve_execution_safety_limits, resolve_selector_with_retries,
    resolve_step_output_paths, route_selector_action, ExecutionSafetyLimits, FunctionRegistry,
    RouteContext, RoutedSelectorAction, RunState, SelectorAction, SelectorArtifactStore,
    SelectorRequest, SelectorResult, SelectorStatus, StatusResolutionInput, StepAttemptRecord,
    WorkflowRunStore,
};
use direclaw::queue::IncomingMessage;
use direclaw::runtime::{bootstrap_state_root, StatePaths};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use tempfile::tempdir;

fn sample_orchestrator() -> OrchestratorConfig {
    serde_yaml::from_str(sample_orchestrator_yaml()).expect("parse orchestrator")
}

fn sample_orchestrator_yaml() -> &'static str {
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
    limits:
      run_timeout_seconds: 40
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
        limits:
          max_retries: 3
  - id: fix_issue
    version: 1
    limits:
      max_total_iterations: 5
      run_timeout_seconds: 50
    steps:
      - id: plan
        type: agent_task
        agent: worker
        prompt: plan
        outputs: [plan]
        output_files:
          plan: out/plan.md
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
  max_step_timeout_seconds: 20
"#
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
        available_functions: vec![
            "workflow.status".to_string(),
            "workflow.cancel".to_string(),
            "orchestrator.list".to_string(),
        ],
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
        if attempt < 2 {
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
    assert_eq!(success.retries_used, 2);
    assert!(!success.fell_back_to_default_workflow);

    let fallback =
        resolve_selector_with_retries(&orchestrator, &request, |_| Some("{}".to_string()));
    assert!(fallback.fell_back_to_default_workflow);
    assert_eq!(
        fallback.result.selected_workflow.as_deref(),
        Some("engineering_default")
    );

    assert!(state_root
        .join("orchestrator/messages/message-1.json")
        .is_file());
    assert!(state_root
        .join("orchestrator/select/processing/selector-1.json")
        .is_file());
}

#[test]
fn selector_actions_start_workflow_status_and_commands_execute() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let orchestrator = sample_orchestrator();
    let store = WorkflowRunStore::new(&state_root);
    let request = sample_selector_request();

    let start_result = SelectorResult {
        selector_id: "selector-1".to_string(),
        status: SelectorStatus::Selected,
        action: Some(SelectorAction::WorkflowStart),
        selected_workflow: Some("fix_issue".to_string()),
        diagnostics_scope: None,
        function_id: None,
        function_args: None,
        reason: None,
    };

    let functions = FunctionRegistry::with_run_store(
        vec![
            "workflow.status".to_string(),
            "workflow.cancel".to_string(),
            "orchestrator.list".to_string(),
        ],
        store.clone(),
    );

    let started = route_selector_action(
        &request,
        &start_result,
        RouteContext {
            status_input: &StatusResolutionInput {
                explicit_run_id: None,
                inbound_workflow_run_id: None,
                channel_profile_id: Some("engineering".to_string()),
                conversation_id: Some("thread-1".to_string()),
            },
            active_conversation_runs: &BTreeMap::new(),
            functions: &functions,
            run_store: &store,
            orchestrator: &orchestrator,
            source_message_id: Some("message-1"),
            now: 100,
        },
    )
    .expect("start route");

    let run_id = match started {
        RoutedSelectorAction::WorkflowStart {
            run_id,
            workflow_id,
        } => {
            assert_eq!(workflow_id, "fix_issue");
            run_id
        }
        other => panic!("unexpected route: {other:?}"),
    };

    let run = store.load_run(&run_id).expect("load run");
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.source_message_id.as_deref(), Some("message-1"));
    assert_eq!(run.selector_id.as_deref(), Some("selector-1"));
    assert_eq!(run.selected_workflow.as_deref(), Some("fix_issue"));
    assert_eq!(run.status_conversation_id.as_deref(), Some("thread-1"));

    let before = store.load_run(&run_id).expect("before status");
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

    let status = route_selector_action(
        &request,
        &status_result,
        RouteContext {
            status_input: &StatusResolutionInput {
                explicit_run_id: Some(run_id.clone()),
                inbound_workflow_run_id: None,
                channel_profile_id: Some("engineering".to_string()),
                conversation_id: Some("thread-1".to_string()),
            },
            active_conversation_runs: &BTreeMap::new(),
            functions: &functions,
            run_store: &store,
            orchestrator: &orchestrator,
            source_message_id: Some("message-1"),
            now: 101,
        },
    )
    .expect("status route");

    match status {
        RoutedSelectorAction::WorkflowStatus {
            run_id: Some(found),
            progress: Some(progress),
            ..
        } => {
            assert_eq!(found, run_id);
            assert_eq!(progress.state, RunState::Running);
        }
        other => panic!("unexpected status route: {other:?}"),
    }

    let after = store.load_run(&run_id).expect("after status");
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
            Value::String(run_id.clone()),
        )])),
        reason: None,
    };
    let routed_command = route_selector_action(
        &request,
        &command_result,
        RouteContext {
            status_input: &StatusResolutionInput {
                explicit_run_id: None,
                inbound_workflow_run_id: None,
                channel_profile_id: Some("engineering".to_string()),
                conversation_id: Some("thread-1".to_string()),
            },
            active_conversation_runs: &BTreeMap::new(),
            functions: &functions,
            run_store: &store,
            orchestrator: &orchestrator,
            source_message_id: Some("message-1"),
            now: 102,
        },
    )
    .expect("command route");
    match routed_command {
        RoutedSelectorAction::CommandInvoke { result } => {
            assert_eq!(result["runId"], Value::String(run_id));
            assert!(result["progress"].is_object());
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
fn run_state_and_progress_are_persisted_at_spec_paths_with_lifecycle_updates() {
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
    store
        .mark_step_attempt_started(&mut run, "plan", 1, 102)
        .expect("step start");

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
    assert!(state_root.join("workflows/runs/run-2.json").is_file());
    assert!(state_root
        .join("workflows/runs/run-2/progress.json")
        .is_file());
    assert!(state_root
        .join("workflows/runs/run-2/steps/plan/attempts/1/result.json")
        .is_file());

    store
        .heartbeat_tick(&run, 104, "heartbeat")
        .expect("heartbeat");

    let progress = store.load_progress("run-2").expect("progress");
    assert_eq!(progress.last_progress_at, 104);

    let invalid = store.transition_state(&mut run, RunState::Queued, 105, "illegal", false, "none");
    assert!(invalid.is_err());
}

#[test]
fn step_execution_contract_enforces_envelope_routing_and_configured_safety_controls() {
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

    let limits = resolve_execution_safety_limits(&orchestrator, workflow, plan);
    assert_eq!(
        limits,
        ExecutionSafetyLimits {
            max_total_iterations: 5,
            run_timeout_seconds: 50,
            step_timeout_seconds: 20,
            max_retries: 2,
        }
    );

    let mut run = WorkflowRunStore::new(tempdir().expect("tempdir").path())
        .create_run("run-3", "fix_issue", 0)
        .expect("run");
    run.state = RunState::Running;
    run.total_iterations = 5;
    assert!(enforce_execution_safety(&run, limits, 2, 0, 1).is_err());

    run.total_iterations = 0;
    assert!(enforce_execution_safety(&run, limits, 60, 0, 1).is_err());
    assert!(enforce_execution_safety(&run, limits, 21, 0, 1).is_err());
    assert!(enforce_execution_safety(&run, limits, 2, 0, 4).is_err());
}

#[test]
fn diagnostics_and_queue_dispatch_paths_are_supported() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let settings_yaml = format!(
        r#"
workspace_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  engineering_orchestrator:
    private_workspace: {orchestrator_ws}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: engineering_orchestrator
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display(),
        orchestrator_ws = dir.path().join("orch").display()
    );
    let settings: Settings = serde_yaml::from_str(&settings_yaml).expect("settings");

    let orch_workspace = dir.path().join("orch");
    fs::create_dir_all(&orch_workspace).expect("orch workspace");
    fs::write(
        orch_workspace.join("orchestrator.yaml"),
        sample_orchestrator_yaml(),
    )
    .expect("write orchestrator");

    let store = WorkflowRunStore::new(&state_root);
    let mut run = store
        .create_run("run-diag", "fix_issue", 5)
        .expect("create run");
    store
        .transition_state(&mut run, RunState::Running, 6, "running", false, "continue")
        .expect("transition");

    let functions = FunctionRegistry::with_run_store(
        vec![
            "workflow.status".to_string(),
            "workflow.cancel".to_string(),
            "orchestrator.list".to_string(),
        ],
        store.clone(),
    );

    let request = sample_selector_request();
    let diag_result = SelectorResult {
        selector_id: "selector-1".to_string(),
        status: SelectorStatus::Selected,
        action: Some(SelectorAction::DiagnosticsInvestigate),
        selected_workflow: None,
        diagnostics_scope: Some(Map::from_iter([(
            "runId".to_string(),
            Value::String("run-diag".to_string()),
        )])),
        function_id: None,
        function_args: None,
        reason: None,
    };

    let diag = route_selector_action(
        &request,
        &diag_result,
        RouteContext {
            status_input: &StatusResolutionInput {
                explicit_run_id: None,
                inbound_workflow_run_id: None,
                channel_profile_id: Some("engineering".to_string()),
                conversation_id: Some("thread-1".to_string()),
            },
            active_conversation_runs: &BTreeMap::new(),
            functions: &functions,
            run_store: &store,
            orchestrator: &sample_orchestrator(),
            source_message_id: Some("message-1"),
            now: 200,
        },
    )
    .expect("diag route");

    match diag {
        RoutedSelectorAction::DiagnosticsInvestigate {
            run_id: Some(found),
            findings,
        } => {
            assert_eq!(found, "run-diag");
            assert!(findings.contains("Diagnostics summary"));
        }
        other => panic!("unexpected diagnostics route: {other:?}"),
    }

    assert!(state_root
        .join("orchestrator/diagnostics/context/diag-selector-1-200.json")
        .is_file());
    assert!(state_root
        .join("orchestrator/diagnostics/results/diag-selector-1-200.json")
        .is_file());

    let mut active = BTreeMap::new();
    active.insert(
        ("engineering".to_string(), "thread-1".to_string()),
        "run-diag".to_string(),
    );

    let action = process_queued_message(
        &state_root,
        &settings,
        &sample_incoming(),
        300,
        &active,
        &functions,
        |attempt, _request, _orchestrator| {
            if attempt == 0 {
                Some("not-json".to_string())
            } else {
                Some(
                    r#"{
                  "selectorId":"sel-message-1",
                  "status":"selected",
                  "action":"workflow_status"
                }"#
                    .to_string(),
                )
            }
        },
    )
    .expect("dispatch");

    match action {
        RoutedSelectorAction::WorkflowStatus {
            run_id: Some(run_id),
            progress: Some(progress),
            ..
        } => {
            assert_eq!(run_id, "run-diag");
            assert_eq!(progress.run_id, "run-diag");
        }
        other => panic!("unexpected dispatched route: {other:?}"),
    }
}

#[test]
fn malicious_output_file_template_is_blocked_before_step_execution() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let mut orchestrator = sample_orchestrator();
    let workflow = orchestrator
        .workflows
        .iter_mut()
        .find(|w| w.id == "fix_issue")
        .expect("workflow");
    let step = workflow
        .steps
        .iter_mut()
        .find(|s| s.id == "plan")
        .expect("plan");
    step.output_files = Some(BTreeMap::from_iter([(
        "plan".to_string(),
        "../../escape.md".to_string(),
    )]));

    let err = resolve_step_output_paths(&state_root, "run-malicious", step, 1)
        .expect_err("must block traversal");
    assert!(err.to_string().contains("output path validation failed"));
}

#[test]
fn process_queue_denies_ungranted_workspace_access_and_logs() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let orchestrator_ws = dir.path().join("orch");
    fs::create_dir_all(&orchestrator_ws).expect("create orchestrator ws");
    fs::write(
        orchestrator_ws.join("orchestrator.yaml"),
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
    private_workspace: /tmp/outside
workflows:
  - id: engineering_default
    version: 1
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
"#,
    )
    .expect("write orchestrator");

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspace_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  engineering_orchestrator:
    private_workspace: {orchestrator_ws}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: engineering_orchestrator
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display(),
        orchestrator_ws = orchestrator_ws.display()
    ))
    .expect("settings");

    let store = WorkflowRunStore::new(&state_root);
    let functions = FunctionRegistry::with_run_store(vec!["workflow.status".to_string()], store);
    let err = process_queued_message(
        &state_root,
        &settings,
        &sample_incoming(),
        300,
        &BTreeMap::new(),
        &functions,
        |_attempt, _request, _orchestrator| None,
    )
    .expect_err("workspace check must fail");
    assert!(err.to_string().contains("workspace access denied"));

    let security_log =
        fs::read_to_string(state_root.join("logs/security.log")).expect("read security log");
    assert!(security_log.contains("workspace access denied"));
}

#[test]
fn process_queue_blocks_malicious_output_template_and_logs() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let settings_yaml = format!(
        r#"
workspace_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  engineering_orchestrator:
    private_workspace: {orchestrator_ws}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: engineering_orchestrator
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display(),
        orchestrator_ws = dir.path().join("orch").display()
    );
    let settings: Settings = serde_yaml::from_str(&settings_yaml).expect("settings");

    let orch_workspace = dir.path().join("orch");
    fs::create_dir_all(&orch_workspace).expect("orch workspace");
    fs::write(
        orch_workspace.join("orchestrator.yaml"),
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: fix_issue
selection_max_retries: 2
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.2
workflows:
  - id: fix_issue
    version: 1
    steps:
      - id: plan
        type: agent_task
        agent: worker
        prompt: plan
        outputs: [plan]
        output_files:
          plan: ../../escape.md
"#,
    )
    .expect("write orchestrator");

    let functions = FunctionRegistry::with_run_store(
        vec![
            "workflow.status".to_string(),
            "workflow.cancel".to_string(),
            "orchestrator.list".to_string(),
        ],
        WorkflowRunStore::new(&state_root),
    );

    let err = process_queued_message(
        &state_root,
        &settings,
        &sample_incoming(),
        300,
        &BTreeMap::new(),
        &functions,
        |_attempt, _request, _orchestrator| {
            Some(
                r#"{
              "selectorId":"sel-message-1",
              "status":"selected",
              "action":"workflow_start",
              "selectedWorkflow":"fix_issue"
            }"#
                .to_string(),
            )
        },
    )
    .expect_err("malicious output path should fail");
    assert!(err.to_string().contains("output path validation failed"));

    let security_log =
        fs::read_to_string(state_root.join("logs/security.log")).expect("read security log");
    assert!(security_log.contains("output path validation denied"));
}
