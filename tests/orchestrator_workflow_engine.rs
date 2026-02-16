use direclaw::config::{OrchestratorConfig, OutputKey, PathTemplate, Settings};
use direclaw::orchestration::function_registry::FunctionRegistry;
use direclaw::orchestration::output_contract::{
    evaluate_step_result, parse_workflow_result_envelope, resolve_step_output_paths,
};
use direclaw::orchestration::prompt_render::render_step_prompt;
use direclaw::orchestration::routing::{process_queued_message, StatusResolutionInput};
use direclaw::orchestration::run_store::{
    RunState, StepAttemptRecord, WorkflowRunRecord, WorkflowRunStore,
};
use direclaw::orchestration::selector::{
    parse_and_validate_selector_result, resolve_selector_with_retries, SelectorAction,
    SelectorRequest, SelectorResult, SelectorStatus,
};
use direclaw::orchestration::selector_artifacts::SelectorArtifactStore;
use direclaw::orchestration::transitions::{
    route_selector_action, RouteContext, RoutedSelectorAction,
};
use direclaw::orchestration::workflow_engine::{
    enforce_execution_safety, resolve_execution_safety_limits, ExecutionSafetyLimits,
    WorkflowEngine,
};
use direclaw::orchestration::workspace_access::resolve_workspace_access_context;
use direclaw::provider::RunnerBinaries;
use direclaw::queue::IncomingMessage;
use direclaw::runtime::{bootstrap_state_root, StatePaths};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
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
        outputs: [summary, artifact]
        output_files:
          summary: out/start-summary.txt
          artifact: out/start-artifact.txt
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
        outputs: [decision, summary, feedback]
        output_files:
          decision: out/review-decision.txt
          summary: out/review-summary.txt
          feedback: out/review-feedback.txt
        on_approve: done
        on_reject: plan
      - id: done
        type: agent_task
        agent: worker
        prompt: done
        outputs: [summary, artifact]
        output_files:
          summary: out/done-summary.txt
          artifact: out/done-artifact.txt
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
        available_function_schemas: FunctionRegistry::new(vec![
            "workflow.status".to_string(),
            "workflow.cancel".to_string(),
            "orchestrator.list".to_string(),
        ])
        .available_function_schemas(),
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

fn write_script(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn mock_runner_binaries(root: &Path) -> RunnerBinaries {
    let anthropic = root.join("claude-mock");
    let openai = root.join("codex-mock");
    write_script(
        &anthropic,
        "#!/bin/sh\necho '[workflow_result]{\"decision\":\"approve\",\"summary\":\"ok\",\"feedback\":\"none\",\"plan\":\"ok\",\"artifact\":\"ok\"}[/workflow_result]'\n",
    );
    write_script(
        &openai,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"decision\\\":\\\"approve\\\",\\\"summary\\\":\\\"ok\\\",\\\"feedback\\\":\\\"none\\\",\\\"plan\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );
    RunnerBinaries {
        anthropic: anthropic.display().to_string(),
        openai: openai.display().to_string(),
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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
    let runner_binaries = mock_runner_binaries(dir.path());

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
            workspace_access_context: None,
            runner_binaries: Some(runner_binaries.clone()),
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
    assert_eq!(run.state, RunState::Succeeded);
    assert_eq!(run.source_message_id.as_deref(), Some("message-1"));
    assert_eq!(run.selector_id.as_deref(), Some("selector-1"));
    assert_eq!(run.selected_workflow.as_deref(), Some("fix_issue"));
    assert_eq!(run.status_conversation_id.as_deref(), Some("thread-1"));
    assert_eq!(
        run.inputs.get("user_message"),
        Some(&Value::String("please fix this bug".to_string()))
    );
    assert!(state_root
        .join(format!(
            "workflows/runs/{run_id}/steps/plan/attempts/1/result.json"
        ))
        .is_file());
    let plan_attempt = store
        .load_step_attempt(&run_id, "plan", 1)
        .expect("load plan attempt");
    let plan_output_file = plan_attempt
        .output_files
        .get("plan")
        .expect("plan output file path");
    assert!(Path::new(plan_output_file).is_file());
    assert_eq!(
        fs::read_to_string(plan_output_file).expect("read plan output"),
        "ok"
    );
    assert_eq!(plan_attempt.next_step_id.as_deref(), Some("review"));
    assert!(state_root
        .join(format!(
            "workflows/runs/{run_id}/steps/plan/attempts/1/provider_invocation.json"
        ))
        .is_file());
    assert!(state_root
        .join(format!(
            "workflows/runs/{run_id}/steps/plan/attempts/1/provider_prompts/{run_id}-plan-1_prompt.md"
        ))
        .is_file());
    assert!(state_root
        .join(format!(
            "workflows/runs/{run_id}/steps/plan/attempts/1/provider_prompts/{run_id}-plan-1_context.md"
        ))
        .is_file());
    assert!(state_root
        .join(format!(
            "workflows/runs/{run_id}/steps/review/attempts/1/result.json"
        ))
        .is_file());
    assert!(state_root
        .join(format!(
            "workflows/runs/{run_id}/steps/done/attempts/1/result.json"
        ))
        .is_file());
    let engine_log_path = state_root.join(format!("workflows/runs/{run_id}/engine.log"));
    assert!(engine_log_path.is_file());
    let engine_log = fs::read_to_string(engine_log_path).expect("read engine log");
    assert!(engine_log.contains(&format!("run_id={run_id}")));
    assert!(engine_log.contains("step_id=plan"));
    assert!(engine_log.contains("attempt=1"));

    let progress = store.load_progress(&run_id).expect("progress");
    assert_eq!(progress.input_count, run.inputs.len());
    assert!(progress.input_keys.contains(&"user_message".to_string()));
    assert_eq!(
        run.terminal_reason.as_deref(),
        Some("step done attempt 1 finished")
    );

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
            workspace_access_context: None,
            runner_binaries: Some(runner_binaries.clone()),
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
            assert_eq!(progress.state, RunState::Succeeded);
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
            workspace_access_context: None,
            runner_binaries: Some(runner_binaries),
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
fn workflow_engine_resume_replays_persisted_in_progress_attempt() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let store = WorkflowRunStore::new(&state_root);
    let mut run = store
        .create_run("run-resume", "fix_issue", 10)
        .expect("create run");
    store
        .transition_state(
            &mut run,
            RunState::Running,
            11,
            "running",
            false,
            "execute next step",
        )
        .expect("running");
    store
        .mark_step_attempt_started(&mut run, "plan", 1, 12)
        .expect("mark started");

    let engine = WorkflowEngine::new(store.clone(), sample_orchestrator())
        .with_runner_binaries(mock_runner_binaries(dir.path()));
    let resumed = engine.resume("run-resume", 13).expect("resume");
    assert_eq!(resumed.state, RunState::Succeeded);
    assert_eq!(resumed.current_step_id, None);
    assert_eq!(resumed.current_attempt, None);
    assert!(state_root
        .join("workflows/runs/run-resume/steps/plan/attempts/1/result.json")
        .is_file());
    assert!(state_root
        .join("workflows/runs/run-resume/steps/review/attempts/1/result.json")
        .is_file());
    assert!(state_root
        .join("workflows/runs/run-resume/steps/done/attempts/1/result.json")
        .is_file());
}

#[test]
fn workflow_engine_start_failure_transitions_run_to_failed() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let store = WorkflowRunStore::new(&state_root);
    store
        .create_run("run-bad", "missing_workflow", 20)
        .expect("create run");

    let engine = WorkflowEngine::new(store.clone(), sample_orchestrator());
    let err = engine.start("run-bad", 21).expect_err("start should fail");
    assert!(err.to_string().contains("not declared"));

    let failed = store.load_run("run-bad").expect("load failed run");
    assert_eq!(failed.state, RunState::Failed);
    let progress = store.load_progress("run-bad").expect("progress");
    assert_eq!(progress.state, RunState::Failed);
    assert!(progress.summary.contains("engine start failed"));
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
        output_files: BTreeMap::from_iter([("plan".to_string(), "/tmp/run-2-plan.md".to_string())]),
        next_step_id: Some("review".to_string()),
        error: None,
        output_validation_errors: BTreeMap::new(),
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
        r#"[workflow_result]{"plan":"plan.md"}[/workflow_result]"#,
        &BTreeMap::new(),
    )
    .expect("plan result");
    assert_eq!(plan_out.next_step_id.as_deref(), Some("review"));

    let reject_out = evaluate_step_result(
        workflow,
        review,
        r#"[workflow_result]{"decision":"reject","summary":"retry","feedback":"missing tests"}[/workflow_result]"#,
        &BTreeMap::new(),
    )
    .expect("review reject");
    assert_eq!(reject_out.next_step_id.as_deref(), Some("plan"));

    let approve_out = evaluate_step_result(
        workflow,
        review,
        r#"[workflow_result]{"decision":"approve","summary":"accepted","feedback":"ok"}[/workflow_result]"#,
        &BTreeMap::new(),
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
fn execution_safety_limit_precedence_prefers_workflow_and_step_over_defaults() {
    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: eng
selector_agent: router
default_workflow: wf_a
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: wf_a
    version: 1
    limits:
      max_total_iterations: 9
    steps:
      - id: s1
        type: agent_task
        agent: worker
        prompt: hi
        outputs: [result]
        output_files:
          result: out/s1-result.txt
        limits:
          max_retries: 7
workflow_orchestration:
  max_total_iterations: 4
  default_run_timeout_seconds: 33
  default_step_timeout_seconds: 25
  max_step_timeout_seconds: 10
"#,
    )
    .expect("orchestrator");
    let workflow = orchestrator.workflows.first().expect("workflow");
    let step = workflow.steps.first().expect("step");
    let limits = resolve_execution_safety_limits(&orchestrator, workflow, step);
    assert_eq!(
        limits,
        ExecutionSafetyLimits {
            max_total_iterations: 9,
            run_timeout_seconds: 33,
            step_timeout_seconds: 10,
            max_retries: 7,
        }
    );
}

#[test]
fn max_iterations_violation_fails_run_with_explicit_reason() {
    let dir = tempdir().expect("tempdir");
    let reject = dir.path().join("claude-reject");
    write_script(
        &reject,
        "#!/bin/sh\necho '[workflow_result]{\"decision\":\"reject\"}[/workflow_result]'\n",
    );
    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: wf
selection_max_retries: 1
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: wf
    version: 1
    limits:
      max_total_iterations: 2
    steps:
      - id: review
        type: agent_review
        agent: worker
        prompt: review
        outputs: [decision]
        output_files:
          decision: out/review-decision.txt
        on_approve: done
        on_reject: review
      - id: done
        type: agent_task
        agent: worker
        prompt: done
        outputs: [decision]
        output_files:
          decision: out/done-decision.txt
"#,
    )
    .expect("orchestrator");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let store = WorkflowRunStore::new(&state_root);
    store.create_run("run-max-iters", "wf", 1).expect("run");
    let engine =
        WorkflowEngine::new(store.clone(), orchestrator).with_runner_binaries(RunnerBinaries {
            anthropic: reject.display().to_string(),
            openai: "unused".to_string(),
        });

    let err = engine.start("run-max-iters", 2).expect_err("must fail");
    assert!(err.to_string().contains("max total iterations"));
    let run = store.load_run("run-max-iters").expect("run");
    assert_eq!(run.state, RunState::Failed);
    assert!(run
        .terminal_reason
        .as_deref()
        .unwrap_or_default()
        .contains("max total iterations"));
}

#[test]
fn run_timeout_violation_fails_run_with_explicit_reason() {
    let dir = tempdir().expect("tempdir");
    let approve = dir.path().join("claude-approve");
    write_script(
        &approve,
        "#!/bin/sh\necho '[workflow_result]{\"result\":\"ok\"}[/workflow_result]'\n",
    );
    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: wf
selection_max_retries: 1
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: wf
    version: 1
    limits:
      run_timeout_seconds: 2
    steps:
      - id: s1
        type: agent_task
        agent: worker
        prompt: test
        outputs: [result]
        output_files:
          result: out/s1-result.txt
"#,
    )
    .expect("orchestrator");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let store = WorkflowRunStore::new(&state_root);
    store.create_run("run-timeout-limit", "wf", 1).expect("run");
    let engine =
        WorkflowEngine::new(store.clone(), orchestrator).with_runner_binaries(RunnerBinaries {
            anthropic: approve.display().to_string(),
            openai: "unused".to_string(),
        });

    let err = engine
        .start("run-timeout-limit", 10)
        .expect_err("must fail on run timeout");
    assert!(err.to_string().contains("run timed out"));
    let run = store.load_run("run-timeout-limit").expect("run");
    assert_eq!(run.state, RunState::Failed);
    assert!(run
        .terminal_reason
        .as_deref()
        .unwrap_or_default()
        .contains("run timed out"));
}

#[test]
fn run_timeout_uses_elapsed_runtime_across_multiple_steps() {
    let dir = tempdir().expect("tempdir");
    let approve_slow = dir.path().join("claude-slow");
    write_script(
        &approve_slow,
        "#!/bin/sh\nsleep 1\necho '[workflow_result]{\"result\":\"ok\"}[/workflow_result]'\n",
    );
    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: wf
selection_max_retries: 1
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: wf
    version: 1
    limits:
      run_timeout_seconds: 2
    steps:
      - id: s1
        type: agent_task
        agent: worker
        prompt: first
        outputs: [result]
        output_files:
          result: out/s1-result.txt
      - id: s2
        type: agent_task
        agent: worker
        prompt: second
        outputs: [result]
        output_files:
          result: out/s2-result.txt
"#,
    )
    .expect("orchestrator");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let store = WorkflowRunStore::new(&state_root);
    let now = now_secs();
    store
        .create_run("run-real-timeout", "wf", now)
        .expect("create run");
    let engine =
        WorkflowEngine::new(store.clone(), orchestrator).with_runner_binaries(RunnerBinaries {
            anthropic: approve_slow.display().to_string(),
            openai: "unused".to_string(),
        });

    let err = engine
        .start("run-real-timeout", now)
        .expect_err("must fail on elapsed run timeout");
    assert!(err.to_string().contains("run timed out"));

    let run = store.load_run("run-real-timeout").expect("run");
    assert_eq!(run.state, RunState::Failed);
    assert!(state_root
        .join("workflows/runs/run-real-timeout/steps/s1/attempts/1/result.json")
        .is_file());
    assert!(state_root
        .join("workflows/runs/run-real-timeout/steps/s2/attempts/1/result.json")
        .is_file());
}

#[test]
fn diagnostics_and_queue_dispatch_paths_are_supported() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let settings_yaml = format!(
        r#"
workspaces_path: {workspace}
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
    let runner_binaries = mock_runner_binaries(dir.path());
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
            workspace_access_context: None,
            runner_binaries: Some(runner_binaries),
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
    step.output_files = BTreeMap::from_iter([(
        OutputKey::parse_output_file_key("plan").expect("valid key"),
        PathTemplate::parse("../../escape.md").expect("valid template"),
    )]);

    let err = resolve_step_output_paths(&state_root, "run-malicious", step, 1)
        .expect_err("must block traversal");
    assert!(err.to_string().contains("output path validation failed"));
}

#[test]
fn command_invoke_validation_enforces_schema_and_allowlist() {
    let request = SelectorRequest {
        selector_id: "selector-1".to_string(),
        channel_profile_id: "engineering".to_string(),
        message_id: "message-1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        user_message: "cancel".to_string(),
        available_workflows: vec!["engineering_default".to_string()],
        default_workflow: "engineering_default".to_string(),
        available_functions: vec!["workflow.cancel".to_string()],
        available_function_schemas: FunctionRegistry::new(vec!["workflow.cancel".to_string()])
            .available_function_schemas(),
    };

    let unknown_key = r#"{
      "selectorId":"selector-1",
      "status":"selected",
      "action":"command_invoke",
      "functionId":"workflow.cancel",
      "functionArgs":{"runId":"run-1","extra":"nope"}
    }"#;
    let err = parse_and_validate_selector_result(unknown_key, &request).expect_err("must fail");
    assert!(err.to_string().contains("unknown argument"));

    let wrong_type = r#"{
      "selectorId":"selector-1",
      "status":"selected",
      "action":"command_invoke",
      "functionId":"workflow.cancel",
      "functionArgs":{"runId":10}
    }"#;
    let err = parse_and_validate_selector_result(wrong_type, &request).expect_err("must fail");
    assert!(err.to_string().contains("must be string"));
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
        outputs: [summary, artifact]
        output_files:
          summary: out/start-summary.txt
          artifact: out/start-artifact.txt
"#,
    )
    .expect("write orchestrator");

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
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
workspaces_path: {workspace}
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

#[test]
fn step_prompt_renderer_supports_engineering_and_product_placeholders() {
    let engineering_yaml =
        fs::read_to_string("docs/build/spec/examples/orchestrators/engineering.orchestrator.yaml")
            .expect("read engineering fixture");
    let product_yaml =
        fs::read_to_string("docs/build/spec/examples/orchestrators/product.orchestrator.yaml")
            .expect("read product fixture");
    let engineering: OrchestratorConfig =
        serde_yaml::from_str(&engineering_yaml).expect("parse engineering fixture");
    let product: OrchestratorConfig =
        serde_yaml::from_str(&product_yaml).expect("parse product fixture");

    let engineering_workflow = engineering
        .workflows
        .iter()
        .find(|workflow| workflow.id == "code_with_reviews")
        .expect("workflow");
    let plan_review_step = engineering_workflow
        .steps
        .iter()
        .find(|step| step.id == "plan_review")
        .expect("step");
    let output_paths =
        resolve_step_output_paths(Path::new("/tmp/.direclaw"), "run-1", plan_review_step, 2)
            .expect("output paths");
    let run = WorkflowRunRecord {
        run_id: "run-1".to_string(),
        workflow_id: engineering_workflow.id.clone(),
        state: RunState::Running,
        inputs: Map::from_iter([("channel".to_string(), Value::String("slack".to_string()))]),
        current_step_id: Some("plan_review".to_string()),
        current_attempt: Some(2),
        started_at: 10,
        updated_at: 12,
        total_iterations: 2,
        source_message_id: None,
        selector_id: None,
        selected_workflow: None,
        status_conversation_id: None,
        terminal_reason: None,
    };
    let rendered = render_step_prompt(
        &run,
        engineering_workflow,
        plan_review_step,
        2,
        Path::new("/tmp/workspace/run-1"),
        &output_paths,
        &BTreeMap::from_iter([(
            "plan_create".to_string(),
            Map::from_iter([(
                "plan_doc".to_string(),
                Value::String("Plan body".to_string()),
            )]),
        )]),
    )
    .expect("render engineering");
    assert!(rendered.prompt.contains("Plan body"));
    assert!(rendered
        .prompt
        .contains("review/decision__run-1__plan_review__a2.txt"));
    assert!(!rendered.prompt.contains("{{"));

    let product_workflow = product
        .workflows
        .iter()
        .find(|workflow| workflow.id == "product_default")
        .expect("workflow");
    let product_step = product_workflow.steps.first().expect("step");
    let run = WorkflowRunRecord {
        run_id: "run-2".to_string(),
        workflow_id: product_workflow.id.clone(),
        state: RunState::Running,
        inputs: Map::from_iter([(
            "user_message".to_string(),
            Value::String("Summarize roadmap tradeoffs".to_string()),
        )]),
        current_step_id: Some(product_step.id.clone()),
        current_attempt: Some(1),
        started_at: 20,
        updated_at: 21,
        total_iterations: 1,
        source_message_id: None,
        selector_id: None,
        selected_workflow: None,
        status_conversation_id: None,
        terminal_reason: None,
    };
    let rendered = render_step_prompt(
        &run,
        product_workflow,
        product_step,
        1,
        Path::new("/tmp/workspace/run-2"),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .expect("render product");
    assert!(rendered.prompt.contains("Summarize roadmap tradeoffs"));
}

#[test]
fn step_prompt_renderer_fails_fast_on_missing_required_placeholder() {
    let orchestrator_yaml =
        fs::read_to_string("docs/build/spec/examples/orchestrators/product.orchestrator.yaml")
            .expect("read product fixture");
    let orchestrator: OrchestratorConfig =
        serde_yaml::from_str(&orchestrator_yaml).expect("parse product fixture");
    let workflow = orchestrator
        .workflows
        .iter()
        .find(|workflow| workflow.id == "product_default")
        .expect("workflow");
    let step = workflow.steps.first().expect("step");
    let run = WorkflowRunRecord {
        run_id: "run-missing".to_string(),
        workflow_id: workflow.id.clone(),
        state: RunState::Running,
        inputs: Map::new(),
        current_step_id: Some(step.id.clone()),
        current_attempt: Some(1),
        started_at: 1,
        updated_at: 2,
        total_iterations: 0,
        source_message_id: None,
        selector_id: None,
        selected_workflow: None,
        status_conversation_id: None,
        terminal_reason: None,
    };
    let err = render_step_prompt(
        &run,
        workflow,
        step,
        1,
        Path::new("/tmp/workspace/run"),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .expect_err("missing placeholder must fail");
    assert!(err.to_string().contains("missing required placeholder"));
}

#[test]
fn malformed_envelope_increments_attempts_and_respects_retry_limit() {
    let dir = tempdir().expect("tempdir");
    let bad = dir.path().join("claude-bad");
    write_script(&bad, "#!/bin/sh\necho 'missing envelope'\n");
    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: wf
selection_max_retries: 1
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: wf
    version: 1
    steps:
      - id: s1
        type: agent_task
        agent: worker
        prompt: test
        outputs: [result]
        output_files:
          result: out/s1-result.txt
        limits:
          max_retries: 1
"#,
    )
    .expect("orchestrator");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let store = WorkflowRunStore::new(&state_root);
    store.create_run("run-bad-envelope", "wf", 1).expect("run");
    let engine =
        WorkflowEngine::new(store.clone(), orchestrator).with_runner_binaries(RunnerBinaries {
            anthropic: bad.display().to_string(),
            openai: "unused".to_string(),
        });

    let err = engine
        .start("run-bad-envelope", 2)
        .expect_err("must fail after retries");
    assert!(err.to_string().contains("missing [workflow_result]"));
    let run = store.load_run("run-bad-envelope").expect("run");
    assert_eq!(run.state, RunState::Failed);

    let attempt_1 = store
        .load_step_attempt("run-bad-envelope", "s1", 1)
        .expect("attempt1");
    let attempt_2 = store
        .load_step_attempt("run-bad-envelope", "s1", 2)
        .expect("attempt2");
    assert_eq!(attempt_1.state, "failed_retryable");
    assert_eq!(attempt_2.state, "failed");
}

#[test]
fn output_contract_supports_required_and_optional_keys() {
    let workflow: direclaw::config::WorkflowConfig = serde_yaml::from_str(
        r#"
id: wf
version: 1
steps:
  - id: s1
    type: agent_task
    prompt_type: workflow_result_envelope
    agent: worker
    prompt: test
    outputs: [required_key, optional_key?]
    output_files:
      required_key: out/required.txt
      optional_key: out/optional.txt
"#,
    )
    .expect("workflow");
    let step = workflow.steps.first().expect("step");

    let with_required_only = evaluate_step_result(
        &workflow,
        step,
        r#"[workflow_result]{"required_key":"ok"}[/workflow_result]"#,
        &BTreeMap::new(),
    )
    .expect("required key should pass");
    assert_eq!(
        with_required_only.outputs.get("required_key"),
        Some(&Value::String("ok".to_string()))
    );

    let missing_required = evaluate_step_result(
        &workflow,
        step,
        r#"[workflow_result]{"optional_key":"nice"}[/workflow_result]"#,
        &BTreeMap::new(),
    )
    .expect_err("missing required key should fail");
    assert!(missing_required
        .to_string()
        .contains("missing required output keys"));
}

#[test]
fn missing_required_output_key_retries_and_persists_key_level_validation_errors() {
    let dir = tempdir().expect("tempdir");
    let bad = dir.path().join("claude-missing-output");
    write_script(
        &bad,
        "#!/bin/sh\necho '[workflow_result]{\"summary\":\"x\"}[/workflow_result]'\n",
    );
    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: wf
selection_max_retries: 1
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: wf
    version: 1
    steps:
      - id: s1
        type: agent_task
        agent: worker
        prompt: test
        outputs: [plan]
        output_files:
          plan: out/plan.md
        limits:
          max_retries: 1
"#,
    )
    .expect("orchestrator");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let store = WorkflowRunStore::new(&state_root);
    store
        .create_run("run-missing-output", "wf", 1)
        .expect("run");
    let engine =
        WorkflowEngine::new(store.clone(), orchestrator).with_runner_binaries(RunnerBinaries {
            anthropic: bad.display().to_string(),
            openai: "unused".to_string(),
        });

    let err = engine
        .start("run-missing-output", 2)
        .expect_err("must fail after retries");
    assert!(err.to_string().contains("missing required output keys"));
    let run = store.load_run("run-missing-output").expect("run");
    assert_eq!(run.state, RunState::Failed);
    assert!(run
        .terminal_reason
        .as_deref()
        .unwrap_or_default()
        .contains("missing required output keys"));

    let attempt_1 = store
        .load_step_attempt("run-missing-output", "s1", 1)
        .expect("attempt1");
    let attempt_2 = store
        .load_step_attempt("run-missing-output", "s1", 2)
        .expect("attempt2");
    assert_eq!(attempt_1.state, "failed_retryable");
    assert_eq!(attempt_2.state, "failed");
    assert_eq!(
        attempt_2.output_validation_errors.get("plan"),
        Some(&"missing".to_string())
    );
}

#[test]
fn review_transition_engine_handles_approve_and_reject_paths() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let store = WorkflowRunStore::new(&state_root);

    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: wf
selection_max_retries: 1
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: wf
    version: 1
    steps:
      - id: review
        type: agent_review
        agent: worker
        prompt: review
        outputs: [decision]
        output_files:
          decision: out/review-decision.txt
        on_approve: approved
        on_reject: rejected
      - id: approved
        type: agent_task
        agent: worker
        prompt: approved
        outputs: [decision]
        output_files:
          decision: out/approved-decision.txt
        next: done
      - id: rejected
        type: agent_task
        agent: worker
        prompt: rejected
        outputs: [decision]
        output_files:
          decision: out/rejected-decision.txt
        next: done
      - id: done
        type: agent_task
        agent: worker
        prompt: done
        outputs: [decision]
        output_files:
          decision: out/done-decision.txt
"#,
    )
    .expect("orchestrator");

    for (run_id, decision, expected_step, unexpected_step) in [
        ("run-approve", "approve", "approved", "rejected"),
        ("run-reject", "reject", "rejected", "approved"),
    ] {
        let bin = dir.path().join(format!("claude-{decision}"));
        write_script(
            &bin,
            &format!(
                "#!/bin/sh\necho '[workflow_result]{{\"decision\":\"{decision}\"}}[/workflow_result]'\n"
            ),
        );
        store.create_run(run_id, "wf", 10).expect("run");
        let engine = WorkflowEngine::new(store.clone(), orchestrator.clone()).with_runner_binaries(
            RunnerBinaries {
                anthropic: bin.display().to_string(),
                openai: "unused".to_string(),
            },
        );
        let run = engine.start(run_id, 11).expect("run");
        assert_eq!(run.state, RunState::Succeeded);
        assert!(state_root
            .join(format!(
                "workflows/runs/{run_id}/steps/{expected_step}/attempts/1/result.json"
            ))
            .is_file());
        assert!(!state_root
            .join(format!(
                "workflows/runs/{run_id}/steps/{unexpected_step}/attempts/1/result.json"
            ))
            .is_file());
    }
}

#[test]
fn missing_transition_target_fails_run_with_explicit_error() {
    let dir = tempdir().expect("tempdir");
    let approve = dir.path().join("claude-approve");
    write_script(
        &approve,
        "#!/bin/sh\necho '[workflow_result]{\"decision\":\"approve\"}[/workflow_result]'\n",
    );
    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: wf
selection_max_retries: 1
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: wf
    version: 1
    steps:
      - id: review
        type: agent_review
        agent: worker
        prompt: review
        outputs: [decision]
        output_files:
          decision: out/review-decision.txt
        on_approve: missing_step
        on_reject: rejected
      - id: rejected
        type: agent_task
        agent: worker
        prompt: rejected
        outputs: [decision]
        output_files:
          decision: out/rejected-decision.txt
"#,
    )
    .expect("orchestrator");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let store = WorkflowRunStore::new(&state_root);
    store
        .create_run("run-bad-transition", "wf", 1)
        .expect("run");
    let engine =
        WorkflowEngine::new(store.clone(), orchestrator).with_runner_binaries(RunnerBinaries {
            anthropic: approve.display().to_string(),
            openai: "unused".to_string(),
        });

    let err = engine
        .start("run-bad-transition", 2)
        .expect_err("must fail on invalid transition");
    assert!(err.to_string().contains("transition validation failed"));
    let run = store.load_run("run-bad-transition").expect("run");
    assert_eq!(run.state, RunState::Failed);
}

#[test]
fn step_provider_failures_persist_invocation_logs_and_fail_run() {
    let dir = tempdir().expect("tempdir");
    let non_zero = dir.path().join("claude-fail");
    write_script(&non_zero, "#!/bin/sh\necho boom 1>&2\nexit 7\n");
    let timeout = dir.path().join("claude-timeout");
    write_script(&timeout, "#!/bin/sh\nsleep 1\necho late\n");
    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: wf
selection_max_retries: 1
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: wf
    version: 1
    steps:
      - id: s1
        type: agent_task
        agent: worker
        prompt: test
        outputs: [result]
        output_files:
          result: out/s1-result.txt
        limits:
          max_retries: 0
workflow_orchestration:
  default_step_timeout_seconds: 0
  max_step_timeout_seconds: 0
"#,
    )
    .expect("orchestrator");

    for (name, binary) in [("run-fail", non_zero), ("run-timeout", timeout)] {
        let state_root = dir.path().join(format!(".direclaw-{name}"));
        bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
        let store = WorkflowRunStore::new(&state_root);
        store.create_run(name, "wf", 1).expect("run");
        let engine = WorkflowEngine::new(store.clone(), orchestrator.clone()).with_runner_binaries(
            RunnerBinaries {
                anthropic: binary.display().to_string(),
                openai: "unused".to_string(),
            },
        );
        let err = engine.start(name, 2).expect_err("must fail");
        if name == "run-timeout" {
            assert!(err.to_string().contains("step timed out"));
        }
        let run = store.load_run(name).expect("load run");
        assert_eq!(run.state, RunState::Failed);
        if name == "run-timeout" {
            assert!(run
                .terminal_reason
                .as_deref()
                .unwrap_or_default()
                .contains("step timed out"));
        }
        let log_path = state_root.join(format!(
            "workflows/runs/{name}/steps/s1/attempts/1/provider_invocation.json"
        ));
        assert!(log_path.is_file());
    }
}

#[test]
fn step_workspace_denial_is_logged_and_run_fails_safely() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let private_workspace = dir
        .path()
        .join("workspace")
        .join("engineering_orchestrator");
    let denied_agent_workspace = dir.path().join("outside-agent-workspace");
    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: /tmp/workspace
shared_workspaces: {{}}
orchestrators:
  engineering_orchestrator:
    private_workspace: {}
    shared_access: []
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
        private_workspace.display()
    ))
    .expect("settings");
    let workspace_context = resolve_workspace_access_context(&settings, "engineering_orchestrator")
        .expect("workspace context");
    let orchestrator: OrchestratorConfig = serde_yaml::from_str(&format!(
        r#"
id: engineering_orchestrator
selector_agent: workflow_router
default_workflow: wf
selection_max_retries: 1
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
    private_workspace: {}
workflows:
  - id: wf
    version: 1
    steps:
      - id: s1
        type: agent_task
        agent: worker
        prompt: test
        outputs: [result]
        output_files:
          result: out/s1-result.txt
"#,
        denied_agent_workspace.display()
    ))
    .expect("orchestrator");
    let store = WorkflowRunStore::new(&state_root);
    store.create_run("run-denied", "wf", 1).expect("run");
    let engine = WorkflowEngine::new(store.clone(), orchestrator)
        .with_workspace_access_context(workspace_context)
        .with_runner_binaries(mock_runner_binaries(dir.path()));
    let run_workspace = private_workspace.join("workflows/runs/run-denied/workspace");
    assert!(!run_workspace.exists());
    assert!(!denied_agent_workspace.exists());
    let err = engine.start("run-denied", 2).expect_err("must fail");
    assert!(err.to_string().contains("workspace access denied"));
    let run = store.load_run("run-denied").expect("load run");
    assert_eq!(run.state, RunState::Failed);
    assert!(!run_workspace.exists());
    assert!(!denied_agent_workspace.exists());
    let security_log =
        fs::read_to_string(state_root.join("logs/security.log")).expect("read security");
    assert!(security_log.contains("workspace access denied"));
}

#[test]
fn workflow_result_envelope_rejects_multiple_blocks() {
    let raw = "[workflow_result]{\"a\":1}[/workflow_result] x [workflow_result]{\"b\":2}[/workflow_result]";
    let err = parse_workflow_result_envelope(raw).expect_err("multiple envelopes should fail");
    assert!(err.to_string().contains("multiple [workflow_result]"));
}
