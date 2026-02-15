use crate::config::{load_orchestrator_config, OrchestratorConfig, Settings};
use crate::orchestration::diagnostics::append_security_log;
pub use crate::orchestration::function_registry::{FunctionCall, FunctionRegistry};
use crate::orchestration::run_store::WorkflowRunStore;
use crate::orchestration::selector::{
    resolve_orchestrator_id, resolve_selector_with_retries, SelectorAction, SelectorRequest,
    SelectorResult, SelectorStatus,
};
use crate::orchestration::selector_artifacts::SelectorArtifactStore;
use crate::orchestration::transitions::{
    route_selector_action, RouteContext, RoutedSelectorAction,
};
use crate::orchestration::workflow_engine::{resolve_runner_binaries, WorkflowEngine};
use crate::orchestration::workspace_access::verify_orchestrator_workspace_access;
use crate::orchestrator::OrchestratorError;
use crate::provider::RunnerBinaries;
use crate::queue::IncomingMessage;
use std::collections::BTreeMap;
use std::path::Path;

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
