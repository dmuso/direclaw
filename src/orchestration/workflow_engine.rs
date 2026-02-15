use crate::config::{
    OrchestratorConfig, WorkflowConfig, WorkflowStepConfig, WorkflowStepWorkspaceMode,
};
use crate::orchestration::diagnostics::{
    append_security_log, persist_provider_invocation_log, provider_error_log,
};
use crate::orchestration::output_contract::{
    evaluate_step_result, materialize_output_files, output_validation_errors_for,
    resolve_step_output_paths, StepEvaluation,
};
use crate::orchestration::prompt_render::render_step_prompt;
use crate::orchestration::run_store::{
    RunState, StepAttemptRecord, WorkflowRunRecord, WorkflowRunStore,
};
use crate::orchestration::workspace_access::{
    enforce_workspace_access, resolve_agent_workspace_root, WorkspaceAccessContext,
};
use crate::orchestrator::OrchestratorError;
use crate::provider::{
    consume_reset_flag, run_provider, write_file_backed_prompt, PromptArtifacts, ProviderError,
    ProviderKind, ProviderRequest, RunnerBinaries,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

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

            match self.execute_step_attempt(run, workflow, step, attempt, attempt_started_at) {
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

    fn execute_step_attempt(
        &self,
        run: &WorkflowRunRecord,
        workflow: &WorkflowConfig,
        step: &WorkflowStepConfig,
        attempt: u32,
        now: i64,
    ) -> Result<StepEvaluation, OrchestratorError> {
        let orchestrator_workspace = if let Some(context) = self.workspace_access_context.as_ref() {
            context.private_workspace_root.clone()
        } else {
            self.run_store.state_root().to_path_buf()
        };
        let run_workspace = if let Some(context) = self.workspace_access_context.as_ref() {
            context
                .private_workspace_root
                .join("workflows/runs")
                .join(&run.run_id)
                .join("workspace")
        } else {
            self.run_store
                .state_root()
                .join("workflows/runs")
                .join(&run.run_id)
                .join("workspace")
        };

        let agent = self.orchestrator.agents.get(&step.agent).ok_or_else(|| {
            OrchestratorError::StepExecution {
                step_id: step.id.clone(),
                reason: format!("step references unknown agent `{}`", step.agent),
            }
        })?;
        let agent_workspace = resolve_agent_workspace_root(
            self.workspace_access_context
                .as_ref()
                .map(|ctx| ctx.private_workspace_root.as_path())
                .unwrap_or_else(|| self.run_store.state_root()),
            &step.agent,
            agent,
        );
        let execution_cwd = match step.workspace_mode {
            WorkflowStepWorkspaceMode::OrchestratorWorkspace => orchestrator_workspace.clone(),
            WorkflowStepWorkspaceMode::RunWorkspace => run_workspace.clone(),
            WorkflowStepWorkspaceMode::AgentWorkspace => agent_workspace.clone(),
        };

        if let Some(context) = self.workspace_access_context.as_ref() {
            if let Err(err) = enforce_workspace_access(
                context,
                &[
                    orchestrator_workspace.clone(),
                    run_workspace.clone(),
                    agent_workspace.clone(),
                    execution_cwd.clone(),
                ],
            ) {
                append_security_log(
                    self.run_store.state_root(),
                    &format!(
                        "workspace access denied for run `{}` step `{}`: {}",
                        run.run_id, step.id, err
                    ),
                );
                return Err(err);
            }
        }
        fs::create_dir_all(&orchestrator_workspace)
            .map_err(|err| io_error(&orchestrator_workspace, err))?;
        fs::create_dir_all(&run_workspace).map_err(|err| io_error(&run_workspace, err))?;
        fs::create_dir_all(&agent_workspace).map_err(|err| io_error(&agent_workspace, err))?;
        fs::create_dir_all(&execution_cwd).map_err(|err| io_error(&execution_cwd, err))?;

        let step_outputs =
            load_latest_step_outputs(self.run_store.state_root(), &run.run_id, workflow, &step.id)?;
        let output_paths = match resolve_step_output_paths(
            self.run_store.state_root(),
            &run.run_id,
            step,
            attempt,
        ) {
            Ok(paths) => paths,
            Err(err @ OrchestratorError::OutputPathValidation { .. }) => {
                append_security_log(
                    self.run_store.state_root(),
                    &format!(
                        "output path validation denied for run `{}` step `{}` attempt `{}`: {}",
                        run.run_id, step.id, attempt, err
                    ),
                );
                return Err(err);
            }
            Err(err) => return Err(err),
        };
        let rendered = render_step_prompt(
            run,
            workflow,
            step,
            attempt,
            &run_workspace,
            &output_paths,
            &step_outputs,
        )?;

        let attempt_dir = self
            .run_store
            .state_root()
            .join("workflows/runs")
            .join(&run.run_id)
            .join("steps")
            .join(&step.id)
            .join("attempts")
            .join(attempt.to_string());
        fs::create_dir_all(&attempt_dir).map_err(|err| io_error(&attempt_dir, err))?;

        let artifacts = write_file_backed_prompt(
            &attempt_dir,
            &format!("{}-{}-{attempt}", run.run_id, step.id),
            &rendered.prompt,
            &rendered.context,
        )
        .map_err(|err| OrchestratorError::StepExecution {
            step_id: step.id.clone(),
            reason: err.to_string(),
        })?;

        let reset_flag = execution_cwd.join("reset_flag");
        let reset_resolution =
            consume_reset_flag(&reset_flag).map_err(|err| OrchestratorError::StepExecution {
                step_id: step.id.clone(),
                reason: err.to_string(),
            })?;

        let provider_kind = ProviderKind::try_from(agent.provider.as_str()).map_err(|err| {
            OrchestratorError::StepExecution {
                step_id: step.id.clone(),
                reason: err.to_string(),
            }
        })?;
        let provider_request = ProviderRequest {
            agent_id: step.agent.clone(),
            provider: provider_kind,
            model: agent.model.clone(),
            cwd: execution_cwd.clone(),
            message: provider_instruction_message(&artifacts),
            prompt_artifacts: artifacts.clone(),
            timeout: Duration::from_secs(
                resolve_execution_safety_limits(&self.orchestrator, workflow, step)
                    .step_timeout_seconds,
            ),
            reset_requested: reset_resolution.reset_requested,
            fresh_on_failure: false,
            env_overrides: BTreeMap::new(),
        };

        let step_timeout_seconds =
            resolve_execution_safety_limits(&self.orchestrator, workflow, step)
                .step_timeout_seconds;
        let provider_output =
            run_provider(&provider_request, &self.runner_binaries).map_err(|err| {
                if let Some(log) = provider_error_log(&err) {
                    let _ = persist_provider_invocation_log(&attempt_dir, log);
                }
                match err {
                    ProviderError::Timeout { .. } => OrchestratorError::StepTimeout {
                        step_timeout_seconds,
                    },
                    _ => OrchestratorError::StepExecution {
                        step_id: step.id.clone(),
                        reason: err.to_string(),
                    },
                }
            })?;

        persist_provider_invocation_log(&attempt_dir, &provider_output.log)
            .map_err(|err| io_error(&attempt_dir, err))?;

        let mut evaluation =
            evaluate_step_result(workflow, step, &provider_output.message, &output_paths)?;
        evaluation.output_files =
            materialize_output_files(step, &evaluation.outputs, &output_paths)?;
        self.run_store.append_engine_log(
            &run.run_id,
            now,
            format!(
                "run_id={} step_id={} attempt={} provider={} model={} cwd={}",
                run.run_id,
                step.id,
                attempt,
                provider_output.log.provider,
                provider_output.log.model,
                provider_output.log.working_directory.display(),
            ),
        )?;
        Ok(evaluation)
    }
}

pub fn resolve_runner_binaries() -> RunnerBinaries {
    RunnerBinaries {
        anthropic: std::env::var("DIRECLAW_PROVIDER_BIN_ANTHROPIC")
            .unwrap_or_else(|_| "claude".to_string()),
        openai: std::env::var("DIRECLAW_PROVIDER_BIN_OPENAI")
            .unwrap_or_else(|_| "codex".to_string()),
    }
}

fn elapsed_now(base_now: i64, started_at: Instant) -> i64 {
    base_now.saturating_add(started_at.elapsed().as_secs() as i64)
}

fn io_error(path: &Path, source: std::io::Error) -> OrchestratorError {
    OrchestratorError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn json_error(path: &Path, source: serde_json::Error) -> OrchestratorError {
    OrchestratorError::Json {
        path: path.display().to_string(),
        source,
    }
}

fn provider_instruction_message(artifacts: &PromptArtifacts) -> String {
    let context_paths = artifacts
        .context_files
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Read prompt file at {} and context file(s) at {}. Execute exactly as instructed in those files.",
        artifacts.prompt_file.display(),
        context_paths
    )
}

fn load_latest_step_outputs(
    state_root: &Path,
    run_id: &str,
    workflow: &WorkflowConfig,
    current_step_id: &str,
) -> Result<BTreeMap<String, Map<String, Value>>, OrchestratorError> {
    let mut outputs = BTreeMap::new();
    for step in &workflow.steps {
        if step.id == current_step_id {
            continue;
        }
        let attempts_root = state_root
            .join("workflows/runs")
            .join(run_id)
            .join("steps")
            .join(&step.id)
            .join("attempts");
        if !attempts_root.exists() {
            continue;
        }

        let mut latest_attempt: Option<(u32, StepAttemptRecord)> = None;
        let entries = fs::read_dir(&attempts_root).map_err(|err| io_error(&attempts_root, err))?;
        for entry in entries {
            let entry = entry.map_err(|err| io_error(&attempts_root, err))?;
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            let Ok(attempt_num) = name.parse::<u32>() else {
                continue;
            };
            let result_path = entry.path().join("result.json");
            if !result_path.is_file() {
                continue;
            }
            let raw =
                fs::read_to_string(&result_path).map_err(|err| io_error(&result_path, err))?;
            let attempt: StepAttemptRecord =
                serde_json::from_str(&raw).map_err(|err| json_error(&result_path, err))?;
            if attempt.state != "succeeded" {
                continue;
            }
            match latest_attempt {
                Some((current, _)) if current >= attempt_num => {}
                _ => latest_attempt = Some((attempt_num, attempt)),
            }
        }

        if let Some((_, attempt)) = latest_attempt {
            outputs.insert(step.id.clone(), attempt.outputs);
        }
    }
    Ok(outputs)
}
