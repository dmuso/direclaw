use direclaw::config::WorkflowConfig;
use direclaw::orchestration::run_store::{RunState, WorkflowRunRecord, WorkflowRunStore};
use direclaw::orchestration::workflow_engine::{
    is_retryable_step_error, resolve_next_step_pointer, ExecutionSafetyLimits,
};
use direclaw::orchestrator::OrchestratorError;
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
