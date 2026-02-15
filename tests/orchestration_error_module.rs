use direclaw::orchestration::error::OrchestratorError;
use direclaw::orchestration::run_store::RunState;

#[test]
fn orchestration_error_module_exposes_orchestrator_error() {
    let err = OrchestratorError::InvalidRunTransition {
        from: RunState::Queued,
        to: RunState::Failed,
    };

    assert!(err
        .to_string()
        .contains("workflow run state transition `queued` -> `failed` is invalid"));
}
