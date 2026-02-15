use direclaw::orchestration::progress::ProgressSnapshot;
use direclaw::orchestration::run_store::RunState;

#[test]
fn progress_module_exposes_progress_snapshot_shape() {
    let snapshot = ProgressSnapshot {
        run_id: "run-1".to_string(),
        workflow_id: "wf-default".to_string(),
        state: RunState::Running,
        input_count: 1,
        input_keys: vec!["ticket".to_string()],
        current_step_id: Some("plan".to_string()),
        current_attempt: Some(2),
        started_at: 10,
        updated_at: 20,
        last_progress_at: 20,
        summary: "step plan attempt 2 running".to_string(),
        pending_human_input: false,
        next_expected_action: "await step output".to_string(),
    };

    assert_eq!(snapshot.state, RunState::Running);
    assert_eq!(snapshot.input_keys, vec!["ticket".to_string()]);
}
