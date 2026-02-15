use crate::config::{load_orchestrator_config, ConfigError, OrchestratorConfig, Settings};
#[cfg(test)]
use crate::config::{
    OutputKey, PathTemplate, WorkflowStepConfig, WorkflowStepPromptType, WorkflowStepType,
    WorkflowStepWorkspaceMode,
};
use crate::orchestration::diagnostics::{
    append_security_log, persist_selector_invocation_log, provider_error_log,
};
pub use crate::orchestration::function_registry::{FunctionCall, FunctionRegistry};
pub use crate::orchestration::output_contract::{
    evaluate_step_result, interpolate_output_template, parse_review_decision,
    parse_workflow_result_envelope, resolve_step_output_paths, StepEvaluation,
};
pub use crate::orchestration::prompt_render::{render_step_prompt, StepPromptRender};
pub use crate::orchestration::routing::{resolve_status_run_id, StatusResolutionInput};
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
pub use crate::orchestration::workflow_engine::{
    enforce_execution_safety, is_retryable_step_error, resolve_execution_safety_limits,
    resolve_next_step_pointer, resolve_runner_binaries, ExecutionSafetyLimits, NextStepPointer,
    WorkflowEngine,
};
pub use crate::orchestration::workspace_access::{
    enforce_workspace_access, resolve_agent_workspace_root, resolve_workspace_access_context,
    verify_orchestrator_workspace_access, WorkspaceAccessContext,
};
use crate::provider::{
    run_provider, write_file_backed_prompt, ProviderKind, ProviderRequest, RunnerBinaries,
};
use crate::queue::IncomingMessage;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::time::Duration;

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
