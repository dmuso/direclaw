use crate::config::{OrchestratorConfig, WorkflowConfig, WorkflowStepConfig};
use crate::orchestration::run_store::{WorkflowRunRecord, WorkflowRunStore};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextStepPointer {
    pub step_id: String,
    pub attempt: u32,
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

pub fn resolve_next_step_pointer(
    run_store: &WorkflowRunStore,
    run: &WorkflowRunRecord,
    workflow: &WorkflowConfig,
) -> Result<Option<NextStepPointer>, OrchestratorError> {
    let Some(current_step) = run.current_step_id.as_ref() else {
        return Ok(workflow.steps.first().map(|step| NextStepPointer {
            step_id: step.id.clone(),
            attempt: 1,
        }));
    };

    let Some(current_attempt) = run.current_attempt else {
        return Ok(Some(NextStepPointer {
            step_id: current_step.clone(),
            attempt: 1,
        }));
    };

    let persisted = run_store.load_step_attempt(&run.run_id, current_step, current_attempt);
    match persisted {
        Ok(attempt_record) => {
            if let Some(next) = attempt_record.next_step_id {
                Ok(Some(NextStepPointer {
                    step_id: next,
                    attempt: 1,
                }))
            } else {
                Ok(None)
            }
        }
        Err(OrchestratorError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(Some(NextStepPointer {
                step_id: current_step.clone(),
                attempt: current_attempt,
            }))
        }
        Err(other) => Err(other),
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

pub fn is_retryable_step_error(error: &OrchestratorError) -> bool {
    matches!(
        error,
        OrchestratorError::StepExecution { .. }
            | OrchestratorError::WorkflowEnvelope(_)
            | OrchestratorError::InvalidReviewDecision(_)
            | OrchestratorError::OutputContractValidation { .. }
    )
}
