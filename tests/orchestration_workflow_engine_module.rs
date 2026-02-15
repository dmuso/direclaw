use direclaw::config::{OrchestratorConfig, WorkflowConfig};
use direclaw::orchestration::run_store::{RunState, WorkflowRunRecord, WorkflowRunStore};
use direclaw::orchestration::workflow_engine::{
    is_retryable_step_error, resolve_next_step_pointer, ExecutionSafetyLimits, WorkflowEngine,
};
use direclaw::orchestrator::OrchestratorError;
use direclaw::provider::RunnerBinaries;
use serde_json::Map;
use tempfile::tempdir;

fn sample_workflow() -> WorkflowConfig {
    serde_yaml::from_str(
        r#"
id: fix_issue
version: 1
steps:
  - id: plan
    type: agent_task
    agent: worker
    prompt: plan
    outputs: [plan]
    output_files:
      plan: out/plan.md
  - id: done
    type: agent_task
    agent: worker
    prompt: done
    outputs: [summary]
    output_files:
      summary: out/summary.md
"#,
    )
    .expect("workflow")
}

fn sample_run(run_id: &str) -> WorkflowRunRecord {
    WorkflowRunRecord {
        run_id: run_id.to_string(),
        workflow_id: "fix_issue".to_string(),
        state: RunState::Running,
        inputs: Map::new(),
        current_step_id: None,
        current_attempt: None,
        started_at: 1,
        updated_at: 1,
        total_iterations: 0,
        source_message_id: None,
        selector_id: None,
        selected_workflow: None,
        status_conversation_id: None,
        terminal_reason: None,
    }
}

fn sample_orchestrator() -> OrchestratorConfig {
    serde_yaml::from_str(
        r#"
id: eng
selector_agent: selector
default_workflow: fix_issue
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  selector:
    provider: openai
    model: gpt-4.1
  worker:
    provider: openai
    model: gpt-4.1
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
          plan: out/plan.md
"#,
    )
    .expect("orchestrator")
}

#[test]
fn workflow_engine_module_exposes_execution_safety_defaults() {
    let limits = ExecutionSafetyLimits::default();
    assert_eq!(limits.max_total_iterations, 12);
    assert_eq!(limits.run_timeout_seconds, 3600);
    assert_eq!(limits.step_timeout_seconds, 900);
    assert_eq!(limits.max_retries, 2);
}

#[test]
fn workflow_engine_module_exposes_step_pointer_resolution_and_retryability() {
    let dir = tempdir().expect("tempdir");
    let store = WorkflowRunStore::new(dir.path());
    let workflow = sample_workflow();

    let run = sample_run("run-a");
    let pointer = resolve_next_step_pointer(&store, &run, &workflow)
        .expect("pointer")
        .expect("first step");
    assert_eq!(pointer.step_id, "plan");
    assert_eq!(pointer.attempt, 1);

    let retryable = OrchestratorError::StepExecution {
        step_id: "plan".to_string(),
        reason: "provider failed".to_string(),
    };
    assert!(is_retryable_step_error(&retryable));

    let non_retryable = OrchestratorError::WorkspaceAccessDenied {
        orchestrator_id: "eng".to_string(),
        path: "/tmp".to_string(),
    };
    assert!(!is_retryable_step_error(&non_retryable));
}

#[test]
fn workflow_engine_module_exposes_engine_type_and_builder() {
    let dir = tempdir().expect("tempdir");
    let store = WorkflowRunStore::new(dir.path());
    let orchestrator = sample_orchestrator();
    let binaries = RunnerBinaries {
        anthropic: "claude".to_string(),
        openai: "codex".to_string(),
    };

    let _engine = WorkflowEngine::new(store, orchestrator).with_runner_binaries(binaries);
}
