use crate::config::{OrchestratorConfig, WorkflowConfig, WorkflowStepConfig};
use crate::orchestration::output_contract::output_validation_errors_for;
use crate::orchestration::run_store::{
    RunState, StepAttemptRecord, WorkflowRunRecord, WorkflowRunStore,
};
pub use crate::orchestration::step_execution::resolve_runner_binaries;
use crate::orchestration::step_execution::{execute_step_attempt, StepExecutionContext};
use crate::orchestration::workspace_access::WorkspaceAccessContext;
use crate::orchestrator::OrchestratorError;
use crate::provider::RunnerBinaries;
use serde_json::Map;
use std::collections::BTreeMap;
use std::time::Instant;

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

#[derive(Debug, Clone)]
pub struct WorkflowEngine {
    run_store: WorkflowRunStore,
    orchestrator: OrchestratorConfig,
    runner_binaries: RunnerBinaries,
    workspace_access_context: Option<WorkspaceAccessContext>,
}

impl WorkflowEngine {
    pub fn new(run_store: WorkflowRunStore, orchestrator: OrchestratorConfig) -> Self {
        Self {
            run_store,
            orchestrator,
            runner_binaries: resolve_runner_binaries(),
            workspace_access_context: None,
        }
    }

    pub fn with_runner_binaries(mut self, runner_binaries: RunnerBinaries) -> Self {
        self.runner_binaries = runner_binaries;
        self
    }

    pub fn with_workspace_access_context(
        mut self,
        workspace_access_context: WorkspaceAccessContext,
    ) -> Self {
        self.workspace_access_context = Some(workspace_access_context);
        self
    }

    pub fn start(&self, run_id: &str, now: i64) -> Result<WorkflowRunRecord, OrchestratorError> {
        let mut run = self.run_store.load_run(run_id)?;
        if run.state == RunState::Queued {
            let workflow_id = run.workflow_id.clone();
            self.run_store.transition_state(
                &mut run,
                RunState::Running,
                now,
                format!("workflow {workflow_id} started"),
                false,
                "execute next step",
            )?;
        }
        self.run_until_non_running(&mut run, now.saturating_add(1))?;
        self.run_store.load_run(run_id)
    }

    pub fn resume(&self, run_id: &str, now: i64) -> Result<WorkflowRunRecord, OrchestratorError> {
        let mut run = self.run_store.load_run(run_id)?;
        if run.state.clone().is_terminal() {
            return Ok(run);
        }
        if run.state == RunState::Queued || run.state == RunState::Waiting {
            let workflow_id = run.workflow_id.clone();
            self.run_store.transition_state(
                &mut run,
                RunState::Running,
                now,
                format!("workflow {workflow_id} resumed"),
                false,
                "execute next step",
            )?;
        }
        self.run_until_non_running(&mut run, now.saturating_add(1))?;
        self.run_store.load_run(run_id)
    }

    fn run_until_non_running(
        &self,
        run: &mut WorkflowRunRecord,
        start_now: i64,
    ) -> Result<(), OrchestratorError> {
        // Guard against accidental infinite loops in malformed workflows.
        let max_cycles = 10_000u32;
        let mut cycles = 0u32;
        let run_clock_started = Instant::now();

        while run.state == RunState::Running {
            if cycles >= max_cycles {
                return Err(OrchestratorError::SelectorValidation(
                    "workflow engine exceeded maximum execution cycles".to_string(),
                ));
            }
            let previous_iterations = run.total_iterations;
            let now = elapsed_now(start_now, run_clock_started);
            self.execute_or_fail(run, now)?;
            *run = self.run_store.load_run(&run.run_id)?;
            if run.state == RunState::Running && run.total_iterations == previous_iterations {
                return Err(OrchestratorError::SelectorValidation(
                    "workflow engine made no progress while running".to_string(),
                ));
            }
            cycles = cycles.saturating_add(1);
        }

        Ok(())
    }

    pub fn execute_next(
        &self,
        run: &mut WorkflowRunRecord,
        now: i64,
    ) -> Result<(), OrchestratorError> {
        let workflow = self.workflow_for_run(run)?;
        let Some(pointer) = resolve_next_step_pointer(&self.run_store, run, workflow)? else {
            run.current_step_id = None;
            run.current_attempt = None;
            return self.run_store.transition_state(
                run,
                RunState::Succeeded,
                now,
                "workflow completed",
                false,
                "none",
            );
        };

        let step = workflow
            .steps
            .iter()
            .find(|step| step.id == pointer.step_id)
            .ok_or_else(|| {
                OrchestratorError::SelectorValidation(format!(
                    "workflow `{}` missing step `{}`",
                    workflow.id, pointer.step_id
                ))
            })?;

        let limits = resolve_execution_safety_limits(&self.orchestrator, workflow, step);
        let step_context = StepExecutionContext {
            run_store: &self.run_store,
            orchestrator: &self.orchestrator,
            workspace_access_context: self.workspace_access_context.as_ref(),
            runner_binaries: &self.runner_binaries,
            step_timeout_seconds: limits.step_timeout_seconds,
        };
        let mut attempt = pointer.attempt;
        let step_clock_started = Instant::now();

        loop {
            let attempt_started_at = elapsed_now(now, step_clock_started);
            self.run_store.append_engine_log(
                &run.run_id,
                attempt_started_at,
                format!(
                    "run_id={} decision=execute_next step_id={} attempt={} state={}",
                    run.run_id, step.id, attempt, run.state
                ),
            )?;
            self.run_store
                .mark_step_attempt_started(run, &step.id, attempt, attempt_started_at)?;
            enforce_execution_safety(run, limits, attempt_started_at, attempt_started_at, attempt)?;

            match execute_step_attempt(
                &step_context,
                run,
                workflow,
                step,
                attempt,
                attempt_started_at,
            ) {
                Ok(evaluation) => {
                    let attempt_ended_at = elapsed_now(now, step_clock_started);
                    self.run_store.persist_step_attempt(&StepAttemptRecord {
                        run_id: run.run_id.clone(),
                        step_id: step.id.clone(),
                        attempt,
                        started_at: attempt_started_at,
                        ended_at: attempt_ended_at,
                        state: "succeeded".to_string(),
                        outputs: evaluation.outputs.clone(),
                        output_files: evaluation.output_files.clone(),
                        next_step_id: evaluation.next_step_id.clone(),
                        error: None,
                        output_validation_errors: BTreeMap::new(),
                    })?;
                    *run = self.run_store.load_run(&run.run_id)?;

                    self.run_store.append_engine_log(
                        &run.run_id,
                        attempt_ended_at,
                        format!(
                            "run_id={} step_id={} attempt={} transition=succeeded next={}",
                            run.run_id,
                            step.id,
                            attempt,
                            evaluation
                                .next_step_id
                                .clone()
                                .unwrap_or_else(|| "terminal".to_string())
                        ),
                    )?;
                    enforce_execution_safety(
                        run,
                        limits,
                        attempt_ended_at,
                        attempt_started_at,
                        attempt,
                    )?;

                    if let Some(next) = evaluation.next_step_id {
                        run.current_step_id = Some(next.clone());
                        run.current_attempt = None;
                        self.run_store.checkpoint(
                            run,
                            attempt_ended_at,
                            format!(
                                "step {} attempt {} finished; next {}",
                                step.id, attempt, next
                            ),
                            false,
                            format!("execute step {next}"),
                        )?;
                    } else {
                        run.current_step_id = None;
                        run.current_attempt = None;
                        self.run_store.transition_state(
                            run,
                            RunState::Succeeded,
                            attempt_ended_at,
                            format!("step {} attempt {} finished", step.id, attempt),
                            false,
                            "none",
                        )?;
                    }
                    return Ok(());
                }
                Err(err) => {
                    let attempt_ended_at = elapsed_now(now, step_clock_started);
                    let retryable = is_retryable_step_error(&err);
                    let can_retry = retryable && attempt <= limits.max_retries;
                    let output_validation_errors = output_validation_errors_for(&err);
                    self.run_store.persist_step_attempt(&StepAttemptRecord {
                        run_id: run.run_id.clone(),
                        step_id: step.id.clone(),
                        attempt,
                        started_at: attempt_started_at,
                        ended_at: attempt_ended_at,
                        state: if can_retry {
                            "failed_retryable".to_string()
                        } else {
                            "failed".to_string()
                        },
                        outputs: Map::new(),
                        output_files: BTreeMap::new(),
                        next_step_id: None,
                        error: Some(err.to_string()),
                        output_validation_errors,
                    })?;
                    *run = self.run_store.load_run(&run.run_id)?;
                    self.run_store.append_engine_log(
                        &run.run_id,
                        attempt_ended_at,
                        format!(
                            "run_id={} step_id={} attempt={} transition=failed retryable={} error={}",
                            run.run_id, step.id, attempt, can_retry, err
                        ),
                    )?;
                    enforce_execution_safety(
                        run,
                        limits,
                        attempt_ended_at,
                        attempt_started_at,
                        attempt,
                    )?;
                    if can_retry {
                        attempt = attempt.saturating_add(1);
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    fn execute_or_fail(
        &self,
        run: &mut WorkflowRunRecord,
        now: i64,
    ) -> Result<(), OrchestratorError> {
        match self.execute_next(run, now) {
            Ok(()) => Ok(()),
            Err(err) => {
                let reason = format!("engine start failed: {err}");
                self.run_store.append_engine_log(
                    &run.run_id,
                    now,
                    format!("run_id={} transition=failed reason={reason}", run.run_id),
                )?;
                if !run.state.clone().is_terminal() {
                    self.run_store.transition_state(
                        run,
                        RunState::Failed,
                        now,
                        reason,
                        false,
                        "inspect workflow run artifacts",
                    )?;
                }
                Err(err)
            }
        }
    }

    fn workflow_for_run<'a>(
        &'a self,
        run: &WorkflowRunRecord,
    ) -> Result<&'a WorkflowConfig, OrchestratorError> {
        self.orchestrator
            .workflows
            .iter()
            .find(|workflow| workflow.id == run.workflow_id)
            .ok_or_else(|| {
                OrchestratorError::SelectorValidation(format!(
                    "workflow `{}` is not declared in orchestrator",
                    run.workflow_id
                ))
            })
    }
}

fn elapsed_now(base_now: i64, started_at: Instant) -> i64 {
    base_now.saturating_add(started_at.elapsed().as_secs() as i64)
}
