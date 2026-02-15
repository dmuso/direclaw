use crate::config::{OrchestratorConfig, WorkflowConfig, WorkflowStepConfig};
use crate::orchestration::run_store::WorkflowRunRecord;
use crate::orchestrator::OrchestratorError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionSafetyLimits {
    pub max_total_iterations: u32,
    pub run_timeout_seconds: u64,
    pub step_timeout_seconds: u64,
    pub max_retries: u32,
}

impl Default for ExecutionSafetyLimits {
    fn default() -> Self {
        Self {
            max_total_iterations: 12,
            run_timeout_seconds: 3600,
            step_timeout_seconds: 900,
            max_retries: 2,
        }
    }
}

pub fn resolve_execution_safety_limits(
    orchestrator: &OrchestratorConfig,
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
) -> ExecutionSafetyLimits {
    let defaults = ExecutionSafetyLimits::default();
    let orchestration = orchestrator.workflow_orchestration.as_ref();

    let mut step_timeout_seconds = orchestration
        .and_then(|v| v.default_step_timeout_seconds)
        .unwrap_or(defaults.step_timeout_seconds);
    if let Some(max_step) = orchestration.and_then(|v| v.max_step_timeout_seconds) {
        step_timeout_seconds = step_timeout_seconds.min(max_step);
    }

    ExecutionSafetyLimits {
        max_total_iterations: workflow
            .limits
            .as_ref()
            .and_then(|v| v.max_total_iterations)
            .or_else(|| orchestration.and_then(|v| v.max_total_iterations))
            .unwrap_or(defaults.max_total_iterations),
        run_timeout_seconds: workflow
            .limits
            .as_ref()
            .and_then(|v| v.run_timeout_seconds)
            .or_else(|| orchestration.and_then(|v| v.default_run_timeout_seconds))
            .unwrap_or(defaults.run_timeout_seconds),
        step_timeout_seconds,
        max_retries: step
            .limits
            .as_ref()
            .and_then(|v| v.max_retries)
            .unwrap_or(defaults.max_retries),
    }
}

pub fn enforce_execution_safety(
    run: &WorkflowRunRecord,
    limits: ExecutionSafetyLimits,
    now: i64,
    current_step_started_at: i64,
    current_attempt: u32,
) -> Result<(), OrchestratorError> {
    if run.total_iterations >= limits.max_total_iterations {
        return Err(OrchestratorError::MaxIterationsExceeded {
            max_total_iterations: limits.max_total_iterations,
        });
    }

    if now.saturating_sub(run.started_at) > limits.run_timeout_seconds as i64 {
        return Err(OrchestratorError::RunTimeout {
            run_timeout_seconds: limits.run_timeout_seconds,
        });
    }

    if now.saturating_sub(current_step_started_at) > limits.step_timeout_seconds as i64 {
        return Err(OrchestratorError::StepTimeout {
            step_timeout_seconds: limits.step_timeout_seconds,
        });
    }

    if current_attempt > limits.max_retries + 1 {
        return Err(OrchestratorError::SelectorValidation(format!(
            "retry limit exceeded for step attempt {current_attempt} (max retries {})",
            limits.max_retries
        )));
    }

    Ok(())
}
