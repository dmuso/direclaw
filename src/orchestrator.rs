use crate::commands;
use crate::config::{
    load_orchestrator_config, ConfigError, OrchestratorConfig, Settings, WorkflowConfig,
    WorkflowStepConfig, WorkflowStepWorkspaceMode,
};
#[cfg(test)]
use crate::config::{OutputKey, PathTemplate, WorkflowStepPromptType, WorkflowStepType};
pub use crate::orchestration::output_contract::{
    evaluate_step_result, interpolate_output_template, parse_review_decision,
    parse_workflow_result_envelope, resolve_step_output_paths, StepEvaluation,
};
use crate::orchestration::output_contract::{
    materialize_output_files, output_validation_errors_for,
};
pub use crate::orchestration::prompt_render::{render_step_prompt, StepPromptRender};
pub use crate::orchestration::run_store::{
    ProgressSnapshot, RunState, SelectorStartedRunMetadata, StepAttemptRecord, WorkflowRunRecord,
    WorkflowRunStore,
};
pub use crate::orchestration::selector::{
    parse_and_validate_selector_result, resolve_orchestrator_id, resolve_selector_with_retries,
    FunctionArgSchema, FunctionArgType, FunctionSchema, SelectionResolution, SelectorAction,
    SelectorRequest, SelectorResult, SelectorStatus,
};
pub use crate::orchestration::selector_artifacts::SelectorArtifactStore;
pub use crate::orchestration::workspace_access::{
    enforce_workspace_access, resolve_agent_workspace_root, resolve_workspace_access_context,
    verify_orchestrator_workspace_access, WorkspaceAccessContext,
};
use crate::provider::{
    consume_reset_flag, run_provider, write_file_backed_prompt, InvocationLog, ProviderError,
    ProviderKind, ProviderRequest, RunnerBinaries,
};
use crate::queue::IncomingMessage;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("channel message `{message_id}` is missing `channelProfileId`")]
    MissingChannelProfileId { message_id: String },
    #[error("unknown channel profile `{channel_profile_id}`")]
    UnknownChannelProfileId { channel_profile_id: String },
    #[error("selector result is not valid json: {0}")]
    SelectorJson(String),
    #[error("selector validation failed: {0}")]
    SelectorValidation(String),
    #[error("unknown function id `{function_id}`")]
    UnknownFunction { function_id: String },
    #[error("missing required function argument `{arg}`")]
    MissingFunctionArg { arg: String },
    #[error("unknown function argument `{arg}` for `{function_id}`")]
    UnknownFunctionArg { function_id: String, arg: String },
    #[error("invalid argument type for `{function_id}.{arg}`; expected {expected}")]
    InvalidFunctionArgType {
        function_id: String,
        arg: String,
        expected: String,
    },
    #[error("workflow run `{run_id}` not found")]
    UnknownRunId { run_id: String },
    #[error("workflow run state transition `{from}` -> `{to}` is invalid")]
    InvalidRunTransition { from: RunState, to: RunState },
    #[error("workflow result envelope parse failed: {0}")]
    WorkflowEnvelope(String),
    #[error("workflow review decision must be `approve` or `reject`, got `{0}`")]
    InvalidReviewDecision(String),
    #[error("step prompt render failed for step `{step_id}`: {reason}")]
    StepPromptRender { step_id: String, reason: String },
    #[error("step execution failed for step `{step_id}`: {reason}")]
    StepExecution { step_id: String, reason: String },
    #[error("workflow execution exceeded max total iterations ({max_total_iterations})")]
    MaxIterationsExceeded { max_total_iterations: u32 },
    #[error("workflow run timed out after {run_timeout_seconds}s")]
    RunTimeout { run_timeout_seconds: u64 },
    #[error("workflow step timed out after {step_timeout_seconds}s")]
    StepTimeout { step_timeout_seconds: u64 },
    #[error("workspace access denied for orchestrator `{orchestrator_id}` at path `{path}`")]
    WorkspaceAccessDenied {
        orchestrator_id: String,
        path: String,
    },
    #[error("workspace path validation failed for `{path}`: {reason}")]
    WorkspacePathValidation { path: String, reason: String },
    #[error("output path validation failed for step `{step_id}` template `{template}`: {reason}")]
    OutputPathValidation {
        step_id: String,
        template: String,
        reason: String,
    },
    #[error("step `{step_id}` output contract validation failed: {reason}")]
    OutputContractValidation { step_id: String, reason: String },
    #[error("step `{step_id}` transition validation failed: {reason}")]
    TransitionValidation { step_id: String, reason: String },
    #[error("config error: {0}")]
    Config(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json error at {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

impl From<ConfigError> for OrchestratorError {
    fn from(value: ConfigError) -> Self {
        Self::Config(value.to_string())
    }
}

fn io_error(path: &Path, source: std::io::Error) -> OrchestratorError {
    OrchestratorError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn append_security_log(state_root: &Path, line: &str) {
    let path = state_root.join("logs/security.log");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = file.write_all(format!("{line}\n").as_bytes());
}

fn elapsed_now(base_now: i64, started_at: Instant) -> i64 {
    base_now.saturating_add(started_at.elapsed().as_secs() as i64)
}

fn json_error(path: &Path, source: serde_json::Error) -> OrchestratorError {
    OrchestratorError::Json {
        path: path.display().to_string(),
        source,
    }
}

fn missing_run_for_io(run_id: &str, err: &OrchestratorError) -> Option<OrchestratorError> {
    match err {
        OrchestratorError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            Some(OrchestratorError::UnknownRunId {
                run_id: run_id.to_string(),
            })
        }
        _ => None,
    }
}

pub fn run_selector_attempt_with_provider(
    state_root: &Path,
    settings: &Settings,
    request: &SelectorRequest,
    orchestrator: &OrchestratorConfig,
    attempt: u32,
    binaries: &RunnerBinaries,
) -> Result<String, String> {
    let selector_agent = orchestrator
        .agents
        .get(&orchestrator.selector_agent)
        .ok_or_else(|| {
            format!(
                "selector agent `{}` missing from orchestrator config",
                orchestrator.selector_agent
            )
        })?;
    let provider = ProviderKind::try_from(selector_agent.provider.as_str())
        .map_err(|err| format!("invalid selector provider: {err}"))?;

    let private_workspace = settings
        .resolve_private_workspace(&orchestrator.id)
        .map_err(|err| err.to_string())?;
    let cwd = resolve_agent_workspace_root(
        &private_workspace,
        &orchestrator.selector_agent,
        selector_agent,
    );
    fs::create_dir_all(&cwd).map_err(|err| err.to_string())?;

    let request_json = serde_json::to_string_pretty(request).map_err(|err| err.to_string())?;
    let selector_result_path = cwd
        .join("orchestrator")
        .join("select")
        .join("results")
        .join(format!("{}_attempt_{attempt}.json", request.selector_id));
    if let Some(parent) = selector_result_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let prompt = format!(
        "You are the workflow selector.\nRead this selector request JSON and select the next action.\n{request_json}\n\nInstructions:\n1. Read the selector request from the provided files.\n2. Apply the user message and available workflow/function context.\n3. Output exactly one structured JSON selector result to this path:\n{}\n4. Do not output structured JSON anywhere else and do not rely on stdout.\nDo not use markdown fences.",
        selector_result_path.display()
    );
    let context = format!(
        "orchestratorId={}\nselectorAgent={}\nattempt={attempt}\nselectorResultPath={}",
        orchestrator.id,
        orchestrator.selector_agent,
        selector_result_path.display()
    );
    let request_id = format!("{}_attempt_{attempt}", request.selector_id);
    let artifacts = write_file_backed_prompt(&cwd, &request_id, &prompt, &context)
        .map_err(|err| err.to_string())?;

    let provider_request = ProviderRequest {
        agent_id: orchestrator.selector_agent.clone(),
        provider,
        model: selector_agent.model.clone(),
        cwd: cwd.clone(),
        message: format!(
            "Read [file: {}] and [file: {}]. Write selector result JSON to: {}",
            artifacts.prompt_file.display(),
            artifacts
                .context_files
                .first()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            selector_result_path.display()
        ),
        prompt_artifacts: artifacts,
        timeout: Duration::from_secs(orchestrator.selector_timeout_seconds),
        reset_requested: false,
        fresh_on_failure: false,
        env_overrides: BTreeMap::new(),
    };

    match run_provider(&provider_request, binaries) {
        Ok(result) => {
            persist_selector_invocation_log(
                state_root,
                &request.selector_id,
                attempt,
                Some(&result.log),
                None,
            );
            fs::read_to_string(&selector_result_path).map_err(|err| {
                format!(
                    "selector did not write result file at {}: {}",
                    selector_result_path.display(),
                    err
                )
            })
        }
        Err(err) => {
            let error_text = err.to_string();
            persist_selector_invocation_log(
                state_root,
                &request.selector_id,
                attempt,
                provider_error_log(&err),
                Some(&error_text),
            );
            Err(error_text)
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowEngine {
    run_store: WorkflowRunStore,
    orchestrator: OrchestratorConfig,
    runner_binaries: RunnerBinaries,
    workspace_access_context: Option<WorkspaceAccessContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NextStepPointer {
    step_id: String,
    attempt: u32,
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
        let Some(pointer) = self.resolve_next_step_pointer(run, workflow)? else {
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

    fn resolve_next_step_pointer(
        &self,
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

        let persisted =
            self.run_store
                .load_step_attempt(&run.run_id, current_step, current_attempt);
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

fn provider_instruction_message(artifacts: &crate::provider::PromptArtifacts) -> String {
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

fn resolve_runner_binaries() -> RunnerBinaries {
    RunnerBinaries {
        anthropic: std::env::var("DIRECLAW_PROVIDER_BIN_ANTHROPIC")
            .unwrap_or_else(|_| "claude".to_string()),
        openai: std::env::var("DIRECLAW_PROVIDER_BIN_OPENAI")
            .unwrap_or_else(|_| "codex".to_string()),
    }
}

fn provider_error_log(error: &ProviderError) -> Option<&InvocationLog> {
    match error {
        ProviderError::MissingBinary { log, .. } => Some(log),
        ProviderError::NonZeroExit { log, .. } => Some(log),
        ProviderError::Timeout { log, .. } => Some(log),
        ProviderError::ParseFailure { log, .. } => log.as_deref(),
        ProviderError::UnknownProvider(_)
        | ProviderError::UnsupportedAnthropicModel(_)
        | ProviderError::Io { .. } => None,
    }
}

fn persist_provider_invocation_log(path_root: &Path, log: &InvocationLog) -> std::io::Result<()> {
    let path = path_root.join("provider_invocation.json");
    let payload = Value::Object(Map::from_iter([
        ("agentId".to_string(), Value::String(log.agent_id.clone())),
        (
            "provider".to_string(),
            Value::String(log.provider.to_string()),
        ),
        ("model".to_string(), Value::String(log.model.clone())),
        (
            "commandForm".to_string(),
            Value::String(log.command_form.clone()),
        ),
        (
            "workingDirectory".to_string(),
            Value::String(log.working_directory.display().to_string()),
        ),
        (
            "promptFile".to_string(),
            Value::String(log.prompt_file.display().to_string()),
        ),
        (
            "contextFiles".to_string(),
            Value::Array(
                log.context_files
                    .iter()
                    .map(|path| Value::String(path.display().to_string()))
                    .collect(),
            ),
        ),
        (
            "exitCode".to_string(),
            match log.exit_code {
                Some(value) => Value::from(value),
                None => Value::Null,
            },
        ),
        ("timedOut".to_string(), Value::Bool(log.timed_out)),
    ]));
    let body = serde_json::to_vec_pretty(&payload).map_err(std::io::Error::other)?;
    fs::write(path, body)
}

fn persist_selector_invocation_log(
    state_root: &Path,
    selector_id: &str,
    attempt: u32,
    log: Option<&InvocationLog>,
    error: Option<&str>,
) {
    let path = state_root
        .join("orchestrator/select/logs")
        .join(format!("{selector_id}_attempt_{attempt}.invocation.json"));
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut payload = Map::new();
    payload.insert(
        "selectorId".to_string(),
        Value::String(selector_id.to_string()),
    );
    payload.insert("attempt".to_string(), Value::from(attempt));
    payload.insert("timestamp".to_string(), Value::from(now_secs()));
    payload.insert(
        "status".to_string(),
        Value::String(
            if error.is_some() {
                "failed"
            } else {
                "succeeded"
            }
            .to_string(),
        ),
    );
    if let Some(error) = error {
        payload.insert("error".to_string(), Value::String(error.to_string()));
    }

    if let Some(log) = log {
        payload.insert("agentId".to_string(), Value::String(log.agent_id.clone()));
        payload.insert(
            "provider".to_string(),
            Value::String(log.provider.to_string()),
        );
        payload.insert("model".to_string(), Value::String(log.model.clone()));
        payload.insert(
            "commandForm".to_string(),
            Value::String(log.command_form.clone()),
        );
        payload.insert(
            "workingDirectory".to_string(),
            Value::String(log.working_directory.display().to_string()),
        );
        payload.insert(
            "promptFile".to_string(),
            Value::String(log.prompt_file.display().to_string()),
        );
        payload.insert(
            "contextFiles".to_string(),
            Value::Array(
                log.context_files
                    .iter()
                    .map(|path| Value::String(path.display().to_string()))
                    .collect(),
            ),
        );
        payload.insert(
            "exitCode".to_string(),
            match log.exit_code {
                Some(value) => Value::from(value),
                None => Value::Null,
            },
        );
        payload.insert("timedOut".to_string(), Value::Bool(log.timed_out));
    }

    let body = match serde_json::to_vec_pretty(&Value::Object(payload)) {
        Ok(body) => body,
        Err(_) => return,
    };
    let _ = fs::write(path, body);
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn is_retryable_step_error(error: &OrchestratorError) -> bool {
    matches!(
        error,
        OrchestratorError::StepExecution { .. }
            | OrchestratorError::WorkflowEnvelope(_)
            | OrchestratorError::InvalidReviewDecision(_)
            | OrchestratorError::OutputContractValidation { .. }
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

fn validate_selected_workflow_output_paths(
    state_root: &Path,
    run_id: &str,
    orchestrator: &OrchestratorConfig,
    workflow_id: &str,
) -> Result<(), OrchestratorError> {
    let workflow = orchestrator
        .workflows
        .iter()
        .find(|w| w.id == workflow_id)
        .ok_or_else(|| {
            OrchestratorError::SelectorValidation(format!(
                "workflow `{workflow_id}` is not declared in orchestrator"
            ))
        })?;
    for step in &workflow.steps {
        let _ = resolve_step_output_paths(state_root, run_id, step, 1)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCall {
    pub function_id: String,
    pub args: Map<String, Value>,
}

#[derive(Debug, Clone)]
pub struct FunctionRegistry {
    allowed: BTreeSet<String>,
    catalog: BTreeMap<String, FunctionSchema>,
    run_store: Option<WorkflowRunStore>,
    settings: Option<Settings>,
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self::new(Vec::<String>::new())
    }
}

impl FunctionRegistry {
    fn v1_catalog() -> BTreeMap<String, FunctionSchema> {
        commands::V1_FUNCTIONS
            .iter()
            .map(|def| {
                let args = def
                    .args
                    .iter()
                    .map(|arg| {
                        (
                            arg.name.to_string(),
                            FunctionArgSchema {
                                arg_type: arg.arg_type.into(),
                                required: arg.required,
                                description: arg.description.to_string(),
                            },
                        )
                    })
                    .collect();
                (
                    def.function_id.to_string(),
                    FunctionSchema {
                        function_id: def.function_id.to_string(),
                        description: def.description.to_string(),
                        args,
                        read_only: def.read_only,
                    },
                )
            })
            .collect()
    }

    fn normalize_allowlist<I>(
        function_ids: I,
        catalog: &BTreeMap<String, FunctionSchema>,
    ) -> BTreeSet<String>
    where
        I: IntoIterator<Item = String>,
    {
        function_ids
            .into_iter()
            .filter(|id| catalog.contains_key(id))
            .collect()
    }

    pub fn new<I>(function_ids: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let catalog = Self::v1_catalog();
        Self {
            allowed: Self::normalize_allowlist(function_ids, &catalog),
            catalog,
            run_store: None,
            settings: None,
        }
    }

    pub fn with_run_store<I>(function_ids: I, run_store: WorkflowRunStore) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let catalog = Self::v1_catalog();
        Self {
            allowed: Self::normalize_allowlist(function_ids, &catalog),
            catalog,
            run_store: Some(run_store),
            settings: None,
        }
    }

    pub fn with_context<I>(
        function_ids: I,
        run_store: Option<WorkflowRunStore>,
        settings: Option<Settings>,
    ) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let catalog = Self::v1_catalog();
        Self {
            allowed: Self::normalize_allowlist(function_ids, &catalog),
            catalog,
            run_store,
            settings,
        }
    }

    pub fn v1_defaults(run_store: WorkflowRunStore, settings: &Settings) -> Self {
        let catalog = Self::v1_catalog();
        let allowed = catalog.keys().cloned().collect();
        Self {
            allowed,
            catalog,
            run_store: Some(run_store),
            settings: Some(settings.clone()),
        }
    }

    pub fn contains(&self, function_id: &str) -> bool {
        self.allowed.contains(function_id)
    }

    pub fn available_function_ids(&self) -> Vec<String> {
        self.allowed.iter().cloned().collect()
    }

    pub fn available_function_schemas(&self) -> Vec<FunctionSchema> {
        self.allowed
            .iter()
            .filter_map(|id| self.catalog.get(id))
            .cloned()
            .collect()
    }

    fn validate_args(
        &self,
        call: &FunctionCall,
        schema: &FunctionSchema,
    ) -> Result<(), OrchestratorError> {
        for key in call.args.keys() {
            if !schema.args.contains_key(key) {
                return Err(OrchestratorError::UnknownFunctionArg {
                    function_id: call.function_id.clone(),
                    arg: key.clone(),
                });
            }
        }
        for (arg, arg_schema) in &schema.args {
            match call.args.get(arg) {
                Some(value) => {
                    if !arg_schema.arg_type.matches(value) {
                        return Err(OrchestratorError::InvalidFunctionArgType {
                            function_id: call.function_id.clone(),
                            arg: arg.clone(),
                            expected: arg_schema.arg_type.to_string(),
                        });
                    }
                }
                None if arg_schema.required => {
                    return Err(OrchestratorError::MissingFunctionArg { arg: arg.clone() })
                }
                None => {}
            }
        }
        Ok(())
    }

    pub fn invoke(&self, call: &FunctionCall) -> Result<Value, OrchestratorError> {
        if !self.contains(&call.function_id) {
            return Err(OrchestratorError::UnknownFunction {
                function_id: call.function_id.clone(),
            });
        }
        let schema = self.catalog.get(&call.function_id).ok_or_else(|| {
            OrchestratorError::UnknownFunction {
                function_id: call.function_id.clone(),
            }
        })?;
        self.validate_args(call, schema)?;

        commands::execute_function_invocation(
            &call.function_id,
            &call.args,
            commands::FunctionExecutionContext {
                run_store: self.run_store.as_ref(),
                settings: self.settings.as_ref(),
            },
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResolutionInput {
    pub explicit_run_id: Option<String>,
    pub inbound_workflow_run_id: Option<String>,
    pub channel_profile_id: Option<String>,
    pub conversation_id: Option<String>,
}

fn selector_start_inputs(
    request: &SelectorRequest,
    source_message_id: Option<&str>,
) -> Map<String, Value> {
    let mut inputs = Map::new();
    inputs.insert(
        "user_message".to_string(),
        Value::String(request.user_message.clone()),
    );
    inputs.insert(
        "channel_profile_id".to_string(),
        Value::String(request.channel_profile_id.clone()),
    );
    inputs.insert(
        "selector_id".to_string(),
        Value::String(request.selector_id.clone()),
    );
    inputs.insert(
        "message_id".to_string(),
        Value::String(request.message_id.clone()),
    );
    if let Some(conversation_id) = request.conversation_id.as_ref() {
        inputs.insert(
            "conversation_id".to_string(),
            Value::String(conversation_id.clone()),
        );
    }
    if let Some(source_message_id) = source_message_id {
        inputs.insert(
            "source_message_id".to_string(),
            Value::String(source_message_id.to_string()),
        );
    }
    inputs
}

pub fn resolve_status_run_id(
    input: &StatusResolutionInput,
    active_conversation_runs: &BTreeMap<(String, String), String>,
) -> Option<String> {
    if let Some(explicit) = input
        .explicit_run_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return Some(explicit.clone());
    }

    if let Some(inbound) = input
        .inbound_workflow_run_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return Some(inbound.clone());
    }

    let key = (
        input.channel_profile_id.as_ref()?.to_string(),
        input.conversation_id.as_ref()?.to_string(),
    );
    active_conversation_runs.get(&key).cloned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutedSelectorAction {
    WorkflowStart {
        run_id: String,
        workflow_id: String,
    },
    WorkflowStatus {
        run_id: Option<String>,
        progress: Option<ProgressSnapshot>,
        message: String,
    },
    DiagnosticsInvestigate {
        run_id: Option<String>,
        findings: String,
    },
    CommandInvoke {
        result: Value,
    },
}

pub struct RouteContext<'a> {
    pub status_input: &'a StatusResolutionInput,
    pub active_conversation_runs: &'a BTreeMap<(String, String), String>,
    pub functions: &'a FunctionRegistry,
    pub run_store: &'a WorkflowRunStore,
    pub orchestrator: &'a OrchestratorConfig,
    pub workspace_access_context: Option<WorkspaceAccessContext>,
    pub runner_binaries: Option<RunnerBinaries>,
    pub source_message_id: Option<&'a str>,
    pub now: i64,
}

pub fn route_selector_action(
    request: &SelectorRequest,
    result: &SelectorResult,
    ctx: RouteContext<'_>,
) -> Result<RoutedSelectorAction, OrchestratorError> {
    let validated = parse_and_validate_selector_result(
        &serde_json::to_string(result)
            .map_err(|e| OrchestratorError::SelectorJson(e.to_string()))?,
        request,
    )?;

    let action = validated
        .action
        .ok_or_else(|| OrchestratorError::SelectorValidation("missing action".to_string()))?;

    match action {
        SelectorAction::WorkflowStart => {
            let workflow_id = validated.selected_workflow.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow_start requires selectedWorkflow".to_string(),
                )
            })?;
            if !ctx
                .orchestrator
                .workflows
                .iter()
                .any(|w| w.id == workflow_id)
            {
                let err = OrchestratorError::SelectorValidation(format!(
                    "workflow `{workflow_id}` is not declared in orchestrator"
                ));
                append_security_log(
                    ctx.run_store.state_root(),
                    &format!("workflow_start denied: {err}"),
                );
                return Err(err);
            }

            let run_id = format!("run-{}-{}", request.selector_id, ctx.now);
            if let Err(err) = validate_selected_workflow_output_paths(
                ctx.run_store.state_root(),
                &run_id,
                ctx.orchestrator,
                &workflow_id,
            ) {
                append_security_log(
                    ctx.run_store.state_root(),
                    &format!(
                        "output path validation denied for workflow `{workflow_id}` run `{run_id}`: {err}"
                    ),
                );
                return Err(err);
            }
            ctx.run_store.create_run_with_metadata(
                run_id.clone(),
                workflow_id.clone(),
                SelectorStartedRunMetadata {
                    source_message_id: ctx.source_message_id.map(|v| v.to_string()),
                    selector_id: Some(request.selector_id.clone()),
                    selected_workflow: Some(workflow_id.clone()),
                    status_conversation_id: request.conversation_id.clone(),
                },
                selector_start_inputs(request, ctx.source_message_id),
                ctx.now,
            )?;
            let mut engine = WorkflowEngine::new(ctx.run_store.clone(), ctx.orchestrator.clone());
            if let Some(context) = ctx.workspace_access_context.clone() {
                engine = engine.with_workspace_access_context(context);
            }
            if let Some(binaries) = ctx.runner_binaries.clone() {
                engine = engine.with_runner_binaries(binaries);
            }
            engine.start(&run_id, ctx.now)?;

            Ok(RoutedSelectorAction::WorkflowStart {
                run_id,
                workflow_id,
            })
        }
        SelectorAction::WorkflowStatus => {
            let run_id = resolve_status_run_id(ctx.status_input, ctx.active_conversation_runs);
            let Some(run_id_value) = run_id.clone() else {
                return Ok(RoutedSelectorAction::WorkflowStatus {
                    run_id: None,
                    progress: None,
                    message: "no active workflow run found for this conversation".to_string(),
                });
            };

            match ctx.run_store.load_progress(&run_id_value) {
                Ok(progress) => Ok(RoutedSelectorAction::WorkflowStatus {
                    run_id,
                    progress: Some(progress),
                    message: "workflow progress loaded".to_string(),
                }),
                Err(err) => {
                    let err = missing_run_for_io(&run_id_value, &err).unwrap_or(err);
                    match err {
                        OrchestratorError::UnknownRunId { .. } => {
                            Ok(RoutedSelectorAction::WorkflowStatus {
                                run_id,
                                progress: None,
                                message: format!("workflow run `{run_id_value}` was not found"),
                            })
                        }
                        other => Err(other),
                    }
                }
            }
        }
        SelectorAction::DiagnosticsInvestigate => {
            let explicit_run = validated
                .diagnostics_scope
                .as_ref()
                .and_then(|m| m.get("runId"))
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string());
            let run_id = explicit_run
                .or_else(|| ctx.status_input.inbound_workflow_run_id.clone())
                .or_else(|| {
                    if let (Some(profile), Some(conv)) = (
                        ctx.status_input.channel_profile_id.as_ref(),
                        ctx.status_input.conversation_id.as_ref(),
                    ) {
                        ctx.active_conversation_runs
                            .get(&(profile.clone(), conv.clone()))
                            .cloned()
                    } else {
                        None
                    }
                });

            let diagnostics_id = format!("diag-{}-{}", request.selector_id, ctx.now);
            let diagnostics_root = ctx.run_store.state_root().join("orchestrator/diagnostics");
            fs::create_dir_all(diagnostics_root.join("context"))
                .map_err(|e| io_error(&diagnostics_root, e))?;
            fs::create_dir_all(diagnostics_root.join("results"))
                .map_err(|e| io_error(&diagnostics_root, e))?;
            fs::create_dir_all(diagnostics_root.join("logs"))
                .map_err(|e| io_error(&diagnostics_root, e))?;

            let (findings, context_bundle) = if let Some(run_id_value) = run_id.clone() {
                match ctx.run_store.load_progress(&run_id_value) {
                    Ok(progress) => (
                        format!(
                            "Diagnostics summary for run {}: state={}, summary={}.",
                            run_id_value, progress.state, progress.summary
                        ),
                        Value::Object(Map::from_iter([
                            ("diagnosticsId".to_string(), Value::String(diagnostics_id.clone())),
                            ("runId".to_string(), Value::String(run_id_value.clone())),
                            (
                                "progress".to_string(),
                                serde_json::to_value(progress)
                                    .map_err(|e| OrchestratorError::SelectorJson(e.to_string()))?,
                            ),
                        ])),
                    ),
                    Err(_) => (
                        format!(
                            "Requested diagnostics for run `{run_id_value}`, but no persisted progress was found."
                        ),
                        Value::Object(Map::from_iter([
                            ("diagnosticsId".to_string(), Value::String(diagnostics_id.clone())),
                            ("runId".to_string(), Value::String(run_id_value)),
                            (
                                "note".to_string(),
                                Value::String("run artifacts not found".to_string()),
                            ),
                        ])),
                    ),
                }
            } else {
                (
                    "Diagnostics scope is ambiguous. Which workflow run should I investigate?"
                        .to_string(),
                    Value::Object(Map::from_iter([
                        (
                            "diagnosticsId".to_string(),
                            Value::String(diagnostics_id.clone()),
                        ),
                        (
                            "note".to_string(),
                            Value::String("scope unresolved".to_string()),
                        ),
                    ])),
                )
            };

            let context_path = diagnostics_root
                .join("context")
                .join(format!("{diagnostics_id}.json"));
            let result_path = diagnostics_root
                .join("results")
                .join(format!("{diagnostics_id}.json"));
            let log_path = diagnostics_root
                .join("logs")
                .join(format!("{diagnostics_id}.log"));

            fs::write(
                &context_path,
                serde_json::to_vec_pretty(&context_bundle)
                    .map_err(|e| json_error(&context_path, e))?,
            )
            .map_err(|e| io_error(&context_path, e))?;

            fs::write(
                &result_path,
                serde_json::to_vec_pretty(&Value::Object(Map::from_iter([
                    (
                        "diagnosticsId".to_string(),
                        Value::String(diagnostics_id.clone()),
                    ),
                    ("findings".to_string(), Value::String(findings.clone())),
                ])))
                .map_err(|e| json_error(&result_path, e))?,
            )
            .map_err(|e| io_error(&result_path, e))?;

            fs::write(&log_path, findings.as_bytes()).map_err(|e| io_error(&log_path, e))?;

            Ok(RoutedSelectorAction::DiagnosticsInvestigate { run_id, findings })
        }
        SelectorAction::CommandInvoke => {
            let function_id = validated.function_id.ok_or_else(|| {
                OrchestratorError::SelectorValidation("missing functionId".to_string())
            })?;
            let function_args = validated.function_args.unwrap_or_default();
            let call = FunctionCall {
                function_id,
                args: function_args,
            };
            let invoke_result = ctx.functions.invoke(&call)?;
            Ok(RoutedSelectorAction::CommandInvoke {
                result: invoke_result,
            })
        }
    }
}

pub fn process_queued_message<F>(
    state_root: &Path,
    settings: &Settings,
    inbound: &IncomingMessage,
    now: i64,
    active_conversation_runs: &BTreeMap<(String, String), String>,
    functions: &FunctionRegistry,
    next_selector_attempt: F,
) -> Result<RoutedSelectorAction, OrchestratorError>
where
    F: FnMut(u32, &SelectorRequest, &OrchestratorConfig) -> Option<String>,
{
    process_queued_message_with_runner_binaries(
        state_root,
        settings,
        inbound,
        now,
        active_conversation_runs,
        functions,
        None,
        next_selector_attempt,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn process_queued_message_with_runner_binaries<F>(
    state_root: &Path,
    settings: &Settings,
    inbound: &IncomingMessage,
    now: i64,
    active_conversation_runs: &BTreeMap<(String, String), String>,
    functions: &FunctionRegistry,
    runner_binaries: Option<RunnerBinaries>,
    mut next_selector_attempt: F,
) -> Result<RoutedSelectorAction, OrchestratorError>
where
    F: FnMut(u32, &SelectorRequest, &OrchestratorConfig) -> Option<String>,
{
    let runner_binaries = runner_binaries.unwrap_or_else(resolve_runner_binaries);
    let run_store = WorkflowRunStore::new(state_root);
    let inbound_message = inbound.message.trim().to_ascii_lowercase();
    if let Some(run_id) = inbound
        .workflow_run_id
        .as_ref()
        .filter(|v| !v.trim().is_empty())
    {
        if matches!(
            inbound_message.as_str(),
            "status" | "progress" | "/status" | "/progress"
        ) {
            let status_input = StatusResolutionInput {
                explicit_run_id: Some(run_id.clone()),
                inbound_workflow_run_id: Some(run_id.clone()),
                channel_profile_id: inbound.channel_profile_id.clone(),
                conversation_id: inbound.conversation_id.clone(),
            };

            let pseudo_request = SelectorRequest {
                selector_id: format!("status-{}", inbound.message_id),
                channel_profile_id: inbound.channel_profile_id.clone().unwrap_or_default(),
                message_id: inbound.message_id.clone(),
                conversation_id: inbound.conversation_id.clone(),
                user_message: inbound.message.clone(),
                available_workflows: Vec::new(),
                default_workflow: String::new(),
                available_functions: functions.available_function_ids(),
                available_function_schemas: functions.available_function_schemas(),
            };
            let status_result = SelectorResult {
                selector_id: pseudo_request.selector_id.clone(),
                status: SelectorStatus::Selected,
                action: Some(SelectorAction::WorkflowStatus),
                selected_workflow: None,
                diagnostics_scope: None,
                function_id: None,
                function_args: None,
                reason: None,
            };

            return route_selector_action(
                &pseudo_request,
                &status_result,
                RouteContext {
                    status_input: &status_input,
                    active_conversation_runs,
                    functions,
                    run_store: &run_store,
                    orchestrator: &OrchestratorConfig {
                        id: "status_only".to_string(),
                        selector_agent: "none".to_string(),
                        default_workflow: "none".to_string(),
                        selection_max_retries: 1,
                        selector_timeout_seconds: 30,
                        agents: BTreeMap::new(),
                        workflows: Vec::new(),
                        workflow_orchestration: None,
                    },
                    workspace_access_context: None,
                    runner_binaries: Some(runner_binaries.clone()),
                    source_message_id: Some(&inbound.message_id),
                    now,
                },
            );
        }

        let orchestrator_id = resolve_orchestrator_id(settings, inbound)?;
        let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
        let workspace_context = match verify_orchestrator_workspace_access(
            settings,
            &orchestrator_id,
            &orchestrator,
        ) {
            Ok(context) => context,
            Err(err) => {
                append_security_log(
                        state_root,
                        &format!(
                            "workspace access denied for orchestrator `{orchestrator_id}` message `{}`: {err}",
                            inbound.message_id
                        ),
                    );
                return Err(err);
            }
        };

        let engine = WorkflowEngine::new(run_store.clone(), orchestrator.clone())
            .with_runner_binaries(runner_binaries.clone())
            .with_workspace_access_context(workspace_context);
        let resumed = match engine
            .resume(run_id, now)
            .map_err(|e| missing_run_for_io(run_id, &e).unwrap_or(e))
        {
            Ok(run) => run,
            Err(OrchestratorError::UnknownRunId { .. }) => {
                return Ok(RoutedSelectorAction::WorkflowStatus {
                    run_id: Some(run_id.to_string()),
                    progress: None,
                    message: format!("workflow run `{run_id}` was not found"),
                });
            }
            Err(err) => return Err(err),
        };
        let progress = match run_store
            .load_progress(run_id)
            .map_err(|e| missing_run_for_io(run_id, &e).unwrap_or(e))
        {
            Ok(progress) => progress,
            Err(OrchestratorError::UnknownRunId { .. }) => {
                return Ok(RoutedSelectorAction::WorkflowStatus {
                    run_id: Some(run_id.to_string()),
                    progress: None,
                    message: format!("workflow run `{run_id}` was not found"),
                });
            }
            Err(err) => return Err(err),
        };
        return Ok(RoutedSelectorAction::WorkflowStatus {
            run_id: Some(resumed.run_id),
            progress: Some(progress),
            message: "workflow progress loaded".to_string(),
        });
    }

    let orchestrator_id = resolve_orchestrator_id(settings, inbound)?;
    let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
    let workspace_context =
        match verify_orchestrator_workspace_access(settings, &orchestrator_id, &orchestrator) {
            Ok(context) => context,
            Err(err) => {
                append_security_log(
                    state_root,
                    &format!(
                "workspace access denied for orchestrator `{orchestrator_id}` message `{}`: {err}",
                inbound.message_id
            ),
                );
                return Err(err);
            }
        };

    let request = SelectorRequest {
        selector_id: format!("sel-{}", inbound.message_id),
        channel_profile_id: inbound
            .channel_profile_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        message_id: inbound.message_id.clone(),
        conversation_id: inbound.conversation_id.clone(),
        user_message: inbound.message.clone(),
        available_workflows: orchestrator
            .workflows
            .iter()
            .map(|w| w.id.clone())
            .collect(),
        default_workflow: orchestrator.default_workflow.clone(),
        available_functions: functions.available_function_ids(),
        available_function_schemas: functions.available_function_schemas(),
    };

    let artifact_store = SelectorArtifactStore::new(state_root);
    artifact_store.persist_message_snapshot(inbound)?;
    artifact_store.persist_selector_request(&request)?;
    let _ = artifact_store.move_request_to_processing(&request.selector_id)?;

    let selection = resolve_selector_with_retries(&orchestrator, &request, |attempt| {
        next_selector_attempt(attempt, &request, &orchestrator)
    });
    artifact_store.persist_selector_result(&selection.result)?;
    artifact_store.persist_selector_log(
        &request.selector_id,
        selection
            .result
            .reason
            .as_deref()
            .unwrap_or("selector completed"),
    )?;

    let status_input = StatusResolutionInput {
        explicit_run_id: None,
        inbound_workflow_run_id: inbound.workflow_run_id.clone(),
        channel_profile_id: inbound.channel_profile_id.clone(),
        conversation_id: inbound.conversation_id.clone(),
    };
    route_selector_action(
        &request,
        &selection.result,
        RouteContext {
            status_input: &status_input,
            active_conversation_runs,
            functions,
            run_store: &run_store,
            orchestrator: &orchestrator,
            workspace_access_context: Some(workspace_context),
            runner_binaries: Some(runner_binaries),
            source_message_id: Some(&inbound.message_id),
            now,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    #[test]
    fn resolve_orchestrator_id_from_channel_profile() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  eng:
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: eng
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {}
channels: {}
"#,
        )
        .expect("settings");

        let inbound = IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("engineering".to_string()),
            sender: "dana".to_string(),
            sender_id: "U42".to_string(),
            message: "status?".to_string(),
            timestamp: 1,
            message_id: "m1".to_string(),
            conversation_id: Some("c1".to_string()),
            files: vec![],
            workflow_run_id: None,
            workflow_step_id: None,
        };

        let resolved = resolve_orchestrator_id(&settings, &inbound).expect("resolved");
        assert_eq!(resolved, "eng");
    }

    #[test]
    fn selector_validation_rejects_unknown_function() {
        let request = SelectorRequest {
            selector_id: "sel-1".to_string(),
            channel_profile_id: "engineering".to_string(),
            message_id: "m1".to_string(),
            conversation_id: Some("c1".to_string()),
            user_message: "run command".to_string(),
            available_workflows: vec!["wf".to_string()],
            default_workflow: "wf".to_string(),
            available_functions: vec!["workflow.status".to_string()],
            available_function_schemas: Vec::new(),
        };
        let raw = r#"{
          "selectorId":"sel-1",
          "status":"selected",
          "action":"command_invoke",
          "functionId":"workflow.cancel",
          "functionArgs":{}
        }"#;
        let err = parse_and_validate_selector_result(raw, &request).expect_err("must fail");
        assert!(err.to_string().contains("availableFunctions"));
    }

    #[test]
    fn workflow_result_envelope_parse_and_review_decision() {
        let raw = r#"
ignored
[workflow_result]
{"decision":"approve","feedback":"ok"}
[/workflow_result]
"#;
        let parsed = parse_workflow_result_envelope(raw).expect("parsed");
        let decision = parse_review_decision(&parsed).expect("decision");
        assert!(decision);
    }

    #[test]
    fn run_state_transition_guards_work() {
        assert!(RunState::Queued.can_transition_to(RunState::Running));
        assert!(!RunState::Succeeded.can_transition_to(RunState::Running));
        assert!(!RunState::Failed.can_transition_to(RunState::Running));
    }

    #[test]
    fn workflow_run_record_inputs_round_trip_and_stable_deserialize() {
        let run = WorkflowRunRecord {
            run_id: "run-inputs".to_string(),
            workflow_id: "wf".to_string(),
            state: RunState::Running,
            inputs: Map::from_iter([("ticket".to_string(), Value::String("123".to_string()))]),
            current_step_id: Some("step-1".to_string()),
            current_attempt: Some(1),
            started_at: 10,
            updated_at: 11,
            total_iterations: 1,
            source_message_id: None,
            selector_id: None,
            selected_workflow: None,
            status_conversation_id: None,
            terminal_reason: None,
        };
        let encoded = serde_json::to_string(&run).expect("encode");
        let decoded: WorkflowRunRecord = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(
            decoded.inputs.get("ticket"),
            Some(&Value::String("123".to_string()))
        );

        let minimal = r#"{
          "runId":"run-minimal",
          "workflowId":"wf",
          "state":"queued",
          "startedAt":1,
          "updatedAt":1,
          "totalIterations":0
        }"#;
        let decoded_minimal: WorkflowRunRecord = serde_json::from_str(minimal).expect("minimal");
        assert!(decoded_minimal.inputs.is_empty());
    }

    #[test]
    fn workspace_access_context_and_enforcement_allow_private_and_granted_shared_only() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces:
  docs: /tmp/shared/docs
  finance: /tmp/shared/finance
orchestrators:
  alpha:
    shared_access: [docs]
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("settings");

        let context = resolve_workspace_access_context(&settings, "alpha").expect("context");
        assert_eq!(context.shared_workspace_roots.len(), 1);
        assert!(context.shared_workspace_roots.contains_key("docs"));

        enforce_workspace_access(
            &context,
            &[
                PathBuf::from("/tmp/workspace/alpha/agents/worker"),
                PathBuf::from("/tmp/shared/docs/project/readme.md"),
            ],
        )
        .expect("allowed paths");

        let err = enforce_workspace_access(
            &context,
            &[PathBuf::from("/tmp/shared/finance/budget.xlsx")],
        )
        .expect_err("must deny ungranted shared path");
        assert!(err.to_string().contains("workspace access denied"));
    }

    #[test]
    fn output_path_resolution_interpolates_and_blocks_traversal() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join(".direclaw");

        let step = WorkflowStepConfig {
            id: "plan".to_string(),
            step_type: WorkflowStepType::AgentTask,
            agent: "worker".to_string(),
            prompt: "prompt".to_string(),
            prompt_type: WorkflowStepPromptType::FileOutput,
            workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
            next: None,
            on_approve: None,
            on_reject: None,
            outputs: vec![OutputKey::parse("artifact").expect("valid key")],
            output_files: BTreeMap::from_iter([(
                OutputKey::parse_output_file_key("artifact").expect("valid key"),
                PathTemplate::parse(
                    "artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}.md",
                )
                .expect("valid template"),
            )]),
            limits: None,
        };

        let resolved =
            resolve_step_output_paths(&state_root, "run-123", &step, 2).expect("resolved paths");
        let artifact = resolved.get("artifact").expect("artifact path");
        assert!(artifact
            .starts_with(state_root.join("workflows/runs/run-123/steps/plan/attempts/2/outputs")));
        assert!(artifact
            .display()
            .to_string()
            .ends_with("artifacts/run-123/plan-2.md"));

        let bad_step = WorkflowStepConfig {
            output_files: BTreeMap::from_iter([(
                OutputKey::parse_output_file_key("artifact").expect("valid key"),
                PathTemplate::parse("../escape.md").expect("valid template"),
            )]),
            ..step
        };
        let err =
            resolve_step_output_paths(&state_root, "run-123", &bad_step, 1).expect_err("blocked");
        assert!(err.to_string().contains("output path validation failed"));
    }

    #[test]
    fn function_registry_exposes_machine_readable_schemas_for_v1_scope() {
        let expected_ids = vec![
            "daemon.start",
            "daemon.stop",
            "daemon.restart",
            "daemon.status",
            "daemon.logs",
            "daemon.setup",
            "daemon.send",
            "channels.reset",
            "channels.slack_sync",
            "provider.show",
            "provider.set",
            "model.show",
            "model.set",
            "agent.list",
            "agent.add",
            "agent.show",
            "agent.remove",
            "agent.reset",
            "orchestrator.list",
            "orchestrator.add",
            "orchestrator.show",
            "orchestrator.remove",
            "orchestrator.set_private_workspace",
            "orchestrator.grant_shared_access",
            "orchestrator.revoke_shared_access",
            "orchestrator.set_selector_agent",
            "orchestrator.set_default_workflow",
            "orchestrator.set_selection_max_retries",
            "workflow.list",
            "workflow.show",
            "workflow.add",
            "workflow.remove",
            "workflow.run",
            "workflow.status",
            "workflow.progress",
            "workflow.cancel",
            "channel_profile.list",
            "channel_profile.add",
            "channel_profile.show",
            "channel_profile.remove",
            "channel_profile.set_orchestrator",
            "update.check",
            "update.apply",
            "daemon.attach",
        ];
        let registry = FunctionRegistry::new(expected_ids.iter().map(|id| id.to_string()));
        let schemas = registry.available_function_schemas();
        assert_eq!(schemas.len(), expected_ids.len());
        for expected in &expected_ids {
            assert!(
                schemas.iter().any(|f| &f.function_id == expected),
                "missing function schema for {expected}"
            );
        }
        assert!(schemas
            .iter()
            .any(|f| f.function_id == "workflow.progress" && f.read_only));
        assert!(schemas.iter().any(|f| {
            f.function_id == "workflow.cancel" && !f.read_only && f.args.contains_key("runId")
        }));
        assert!(schemas.iter().any(
            |f| f.function_id == "orchestrator.set_selection_max_retries"
                && f.args.contains_key("count")
        ));
    }

    #[test]
    fn function_registry_rejects_unknown_and_invalid_args() {
        let registry = FunctionRegistry::new(vec!["workflow.status".to_string()]);
        let unknown_arg = FunctionCall {
            function_id: "workflow.status".to_string(),
            args: Map::from_iter([("extra".to_string(), Value::String("x".to_string()))]),
        };
        let err = registry.invoke(&unknown_arg).expect_err("unknown arg");
        assert!(err.to_string().contains("unknown function argument"));

        let invalid_type = FunctionCall {
            function_id: "workflow.status".to_string(),
            args: Map::from_iter([("runId".to_string(), Value::Bool(true))]),
        };
        let err = registry.invoke(&invalid_type).expect_err("invalid type");
        assert!(err.to_string().contains("invalid argument type"));
    }

    #[test]
    fn workflow_status_and_progress_commands_are_read_only() {
        let temp = tempdir().expect("tempdir");
        let store = WorkflowRunStore::new(temp.path());
        let run_id = "run-readonly";
        let mut run = store.create_run(run_id, "wf", 10).expect("create run");
        store
            .transition_state(
                &mut run,
                RunState::Running,
                11,
                "running",
                false,
                "continue",
            )
            .expect("running");
        let before = store.load_run(run_id).expect("before");

        let registry = FunctionRegistry::with_run_store(
            vec![
                "workflow.status".to_string(),
                "workflow.progress".to_string(),
            ],
            store.clone(),
        );
        registry
            .invoke(&FunctionCall {
                function_id: "workflow.status".to_string(),
                args: Map::from_iter([("runId".to_string(), Value::String(run_id.to_string()))]),
            })
            .expect("status call");
        registry
            .invoke(&FunctionCall {
                function_id: "workflow.progress".to_string(),
                args: Map::from_iter([("runId".to_string(), Value::String(run_id.to_string()))]),
            })
            .expect("progress call");

        let after = store.load_run(run_id).expect("after");
        assert_eq!(before.updated_at, after.updated_at);
        assert_eq!(before.state, after.state);
        assert_eq!(before.current_step_id, after.current_step_id);
        assert_eq!(before.current_attempt, after.current_attempt);
    }
}
