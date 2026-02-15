use direclaw::orchestration::workflow_engine::ExecutionSafetyLimits;

#[test]
fn workflow_engine_module_exposes_execution_safety_defaults() {
    let limits = ExecutionSafetyLimits::default();
    assert_eq!(limits.max_total_iterations, 12);
    assert_eq!(limits.run_timeout_seconds, 3600);
    assert_eq!(limits.step_timeout_seconds, 900);
    assert_eq!(limits.max_retries, 2);
}
