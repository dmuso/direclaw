use crate::config::{load_orchestrator_config, OrchestratorConfig, Settings};
use crate::memory::{
    embed_query_text, generate_bulletin_for_message, persist_transcript_observation,
    HybridRecallRequest, MemoryBulletin, MemoryBulletinOptions, MemoryPaths, MemoryRecallOptions,
    MemoryRepository,
};
use crate::orchestration::diagnostics::append_security_log;
use crate::orchestration::error::OrchestratorError;
pub use crate::orchestration::function_registry::{FunctionCall, FunctionRegistry};
use crate::orchestration::run_store::WorkflowRunStore;
use crate::orchestration::scheduler::{
    complete_scheduled_execution, parse_trigger_envelope, ScheduledTriggerEnvelope,
};
use crate::orchestration::selector::{
    resolve_orchestrator_id, resolve_selector_with_retries, run_selector_attempt_with_provider,
    SelectionResolution, SelectorAction, SelectorRequest, SelectorResult, SelectorStatus,
};
use crate::orchestration::selector_artifacts::SelectorArtifactStore;
use crate::orchestration::slack_target::{parse_slack_target_ref, validate_profile_mapping};
use crate::orchestration::transitions::{
    route_selector_action, RouteContext, RoutedSelectorAction,
};
use crate::orchestration::workflow_engine::{resolve_runner_binaries, WorkflowEngine};
use crate::orchestration::workspace_access::verify_orchestrator_workspace_access;
use crate::provider::RunnerBinaries;
use crate::queue::IncomingMessage;
use std::collections::BTreeMap;
use std::path::Path;

fn empty_bulletin(now: i64) -> MemoryBulletin {
    MemoryBulletin {
        rendered: String::new(),
        citations: Vec::new(),
        sections: Vec::new(),
        generated_at: now,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResolutionInput {
    pub explicit_run_id: Option<String>,
    pub inbound_workflow_run_id: Option<String>,
    pub channel_profile_id: Option<String>,
    pub conversation_id: Option<String>,
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
    let orchestrator_id = resolve_orchestrator_id(settings, inbound)?;
    let runtime_root = settings
        .resolve_orchestrator_runtime_root(&orchestrator_id)
        .map_err(|err| OrchestratorError::Config(err.to_string()))?;
    let run_store = WorkflowRunStore::new(&runtime_root);
    if inbound.channel == "scheduler" {
        return route_scheduled_trigger(
            inbound,
            &orchestrator_id,
            settings,
            &runtime_root,
            active_conversation_runs,
            functions,
            &run_store,
            runner_binaries.clone(),
            now,
        );
    }
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
                memory_bulletin: None,
                memory_bulletin_citations: Vec::new(),
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
                    memory_enabled: false,
                    source_message_id: Some(&inbound.message_id),
                    workflow_inputs: None,
                    now,
                },
            );
        }

        let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
        let workspace_context = match verify_orchestrator_workspace_access(
            settings,
            &orchestrator_id,
            &orchestrator,
        ) {
            Ok(context) => context,
            Err(err) => {
                append_security_log(
                        &runtime_root,
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
            .with_workspace_access_context(workspace_context)
            .with_memory_enabled(settings.memory.enabled);
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

    let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
    let workspace_context = match verify_orchestrator_workspace_access(
        settings,
        &orchestrator_id,
        &orchestrator,
    ) {
        Ok(context) => context,
        Err(err) => {
            append_security_log(
                    &runtime_root,
                    &format!(
                        "workspace access denied for orchestrator `{orchestrator_id}` message `{}`: {err}",
                        inbound.message_id
                    ),
                );
            return Err(err);
        }
    };

    let memory_bulletin = if settings.memory.enabled {
        let paths = MemoryPaths::from_runtime_root(&runtime_root);
        let maybe_repo = MemoryRepository::open(&paths.database, &orchestrator_id)
            .and_then(|repo| repo.ensure_schema().map(|_| repo));
        match maybe_repo {
            Ok(repo) => {
                if let Some(conversation_id) = inbound
                    .conversation_id
                    .as_ref()
                    .filter(|value| !value.trim().is_empty())
                {
                    if let Err(err) = persist_transcript_observation(
                        &repo,
                        &orchestrator_id,
                        &inbound.message_id,
                        conversation_id,
                        &inbound.message,
                        now,
                    ) {
                        append_security_log(
                            &runtime_root,
                            &format!(
                                "memory transcript write-back failed for message `{}`: {err}",
                                inbound.message_id
                            ),
                        );
                    }
                }
                let recall_options = MemoryRecallOptions {
                    top_n: settings.memory.retrieval.top_n,
                    rrf_k: settings.memory.retrieval.rrf_k,
                    ..MemoryRecallOptions::default()
                };
                let bulletin_options = MemoryBulletinOptions {
                    max_chars: 4_000,
                    generated_at: now,
                };
                match generate_bulletin_for_message(
                    &repo,
                    &paths,
                    &inbound.message_id,
                    &HybridRecallRequest {
                        requesting_orchestrator_id: orchestrator_id.clone(),
                        conversation_id: inbound.conversation_id.clone(),
                        query_text: inbound.message.clone(),
                        query_embedding: embed_query_text(&inbound.message),
                    },
                    &recall_options,
                    &bulletin_options,
                    Some(&workspace_context),
                ) {
                    Ok(bulletin) => Some(bulletin),
                    Err(err) => {
                        append_security_log(
                            &runtime_root,
                            &format!(
                                "memory bulletin generation failed for message `{}`: {err}",
                                inbound.message_id
                            ),
                        );
                        Some(empty_bulletin(now))
                    }
                }
            }
            Err(err) => {
                append_security_log(
                    &runtime_root,
                    &format!(
                        "memory repository unavailable for message `{}`: {err}",
                        inbound.message_id
                    ),
                );
                Some(empty_bulletin(now))
            }
        }
    } else {
        None
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
        memory_bulletin: memory_bulletin.as_ref().map(|value| value.rendered.clone()),
        memory_bulletin_citations: memory_bulletin
            .as_ref()
            .map(|value| {
                value
                    .citations
                    .iter()
                    .map(|citation| citation.memory_id.clone())
                    .collect()
            })
            .unwrap_or_default(),
        available_workflows: orchestrator
            .workflows
            .iter()
            .map(|w| w.id.clone())
            .collect(),
        default_workflow: orchestrator.default_workflow.clone(),
        available_functions: functions.available_function_ids(),
        available_function_schemas: functions.available_function_schemas(),
    };

    let artifact_store = SelectorArtifactStore::new(&runtime_root);
    artifact_store.persist_message_snapshot(inbound)?;
    artifact_store.persist_selector_request(&request)?;
    let _ = artifact_store.move_request_to_processing(&request.selector_id)?;

    let selection: SelectionResolution =
        resolve_selector_with_retries(&orchestrator, &request, |attempt| {
            next_selector_attempt(attempt, &request, &orchestrator).or_else(|| {
                run_selector_attempt_with_provider(
                    state_root,
                    settings,
                    &request,
                    &orchestrator,
                    attempt,
                    &runner_binaries,
                )
                .ok()
            })
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
            memory_enabled: settings.memory.enabled,
            source_message_id: Some(&inbound.message_id),
            workflow_inputs: None,
            now,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn route_scheduled_trigger(
    inbound: &IncomingMessage,
    orchestrator_id: &str,
    settings: &Settings,
    runtime_root: &Path,
    active_conversation_runs: &BTreeMap<(String, String), String>,
    functions: &FunctionRegistry,
    run_store: &WorkflowRunStore,
    runner_binaries: RunnerBinaries,
    now: i64,
) -> Result<RoutedSelectorAction, OrchestratorError> {
    let envelope: ScheduledTriggerEnvelope =
        parse_trigger_envelope(&inbound.message).map_err(OrchestratorError::SelectorValidation)?;
    if envelope.orchestrator_id != orchestrator_id {
        return Err(OrchestratorError::SelectorValidation(format!(
            "scheduled trigger orchestrator mismatch: expected `{orchestrator_id}`, got `{}`",
            envelope.orchestrator_id
        )));
    }
    let slack_target = envelope
        .target_ref
        .as_ref()
        .map(|value| parse_slack_target_ref(value, "targetRef"))
        .transpose()
        .map_err(OrchestratorError::SelectorValidation)?
        .flatten();
    validate_profile_mapping(settings, orchestrator_id, slack_target.as_ref())
        .map_err(OrchestratorError::SelectorValidation)?;

    let orchestrator = load_orchestrator_config(settings, orchestrator_id)?;
    let workspace_context =
        verify_orchestrator_workspace_access(settings, orchestrator_id, &orchestrator)?;

    let request = SelectorRequest {
        selector_id: format!("schedule-{}", envelope.execution_id),
        channel_profile_id: inbound
            .channel_profile_id
            .clone()
            .unwrap_or_else(|| "scheduler".to_string()),
        message_id: envelope.execution_id.clone(),
        conversation_id: inbound.conversation_id.clone(),
        user_message: format!("scheduled trigger {}", envelope.job_id),
        memory_bulletin: None,
        memory_bulletin_citations: Vec::new(),
        available_workflows: orchestrator
            .workflows
            .iter()
            .map(|workflow| workflow.id.clone())
            .collect(),
        default_workflow: orchestrator.default_workflow.clone(),
        available_functions: functions.available_function_ids(),
        available_function_schemas: functions.available_function_schemas(),
    };

    let mut workflow_inputs = None;
    let result = match envelope.target_action {
        crate::orchestration::scheduler::TargetAction::WorkflowStart {
            workflow_id,
            inputs,
        } => {
            workflow_inputs = Some(inputs);
            SelectorResult {
                selector_id: request.selector_id.clone(),
                status: SelectorStatus::Selected,
                action: Some(SelectorAction::WorkflowStart),
                selected_workflow: Some(workflow_id),
                diagnostics_scope: None,
                function_id: None,
                function_args: None,
                reason: Some(format!(
                    "scheduled_trigger job_id={} execution_id={}",
                    envelope.job_id, envelope.execution_id
                )),
            }
        }
        crate::orchestration::scheduler::TargetAction::CommandInvoke {
            function_id,
            function_args,
        } => SelectorResult {
            selector_id: request.selector_id.clone(),
            status: SelectorStatus::Selected,
            action: Some(SelectorAction::CommandInvoke),
            selected_workflow: None,
            diagnostics_scope: None,
            function_id: Some(function_id),
            function_args: Some(function_args),
            reason: Some(format!(
                "scheduled_trigger job_id={} execution_id={}",
                envelope.job_id, envelope.execution_id
            )),
        },
    };

    match route_selector_action(
        &request,
        &result,
        RouteContext {
            status_input: &StatusResolutionInput {
                explicit_run_id: None,
                inbound_workflow_run_id: inbound.workflow_run_id.clone(),
                channel_profile_id: inbound.channel_profile_id.clone(),
                conversation_id: inbound.conversation_id.clone(),
            },
            active_conversation_runs,
            functions,
            run_store,
            orchestrator: &orchestrator,
            workspace_access_context: Some(workspace_context),
            runner_binaries: Some(runner_binaries),
            memory_enabled: false,
            source_message_id: Some(&inbound.message_id),
            workflow_inputs: workflow_inputs.as_ref(),
            now,
        },
    ) {
        Ok(action) => {
            if let Err(err) = complete_scheduled_execution(
                runtime_root,
                &envelope.job_id,
                &envelope.execution_id,
                true,
                now,
            ) {
                append_security_log(
                    runtime_root,
                    &format!(
                        "scheduled trigger completion failed for job `{}` execution `{}`: {err}",
                        envelope.job_id, envelope.execution_id
                    ),
                );
                return Err(OrchestratorError::ScheduledExecutionCompletion {
                    job_id: envelope.job_id,
                    execution_id: envelope.execution_id,
                    reason: err,
                });
            }
            Ok(action)
        }
        Err(err) => {
            if let Err(completion_err) = complete_scheduled_execution(
                runtime_root,
                &envelope.job_id,
                &envelope.execution_id,
                false,
                now,
            ) {
                append_security_log(
                    runtime_root,
                    &format!(
                        "scheduled trigger completion failed for job `{}` execution `{}` after routing error `{err}`: {completion_err}",
                        envelope.job_id, envelope.execution_id
                    ),
                );
                return Err(OrchestratorError::ScheduledExecutionCompletion {
                    job_id: envelope.job_id,
                    execution_id: envelope.execution_id,
                    reason: format!("routing error: {err}; completion error: {completion_err}"),
                });
            }
            append_security_log(
                runtime_root,
                &format!(
                    "scheduled trigger execution failed for job `{}` execution `{}`: {err}",
                    envelope.job_id, envelope.execution_id
                ),
            );
            Err(err)
        }
    }
}
