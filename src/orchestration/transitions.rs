use crate::config::OrchestratorConfig;
use crate::orchestration::diagnostics::append_security_log;
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::function_registry::{FunctionCall, FunctionRegistry};
use crate::orchestration::output_contract::resolve_step_output_paths;
use crate::orchestration::routing::{resolve_status_run_id, StatusResolutionInput};
use crate::orchestration::run_store::{
    ProgressSnapshot, RunMemoryContext, SelectorStartedRunMetadata, WorkflowRunStore,
};
use crate::orchestration::selector::{
    parse_and_validate_selector_result, SelectorAction, SelectorRequest, SelectorResult,
};
use crate::orchestration::workflow_engine::WorkflowEngine;
use crate::orchestration::workspace_access::WorkspaceAccessContext;
use crate::provider::RunnerBinaries;
use getrandom::getrandom;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

const BASE36_ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const RUN_SUFFIX_SPACE: u32 = 36 * 36 * 36 * 36;
const RUN_ID_MAX_GENERATION_ATTEMPTS: usize = 16;

fn base36_encode_u64(mut value: u64) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let mut chars = Vec::new();
    while value > 0 {
        let idx = (value % 36) as usize;
        chars.push(BASE36_ALPHABET[idx] as char);
        value /= 36;
    }
    chars.iter().rev().collect()
}

fn base36_encode_fixed_u32(mut value: u32, width: usize) -> String {
    let mut chars = vec!['0'; width];
    for idx in (0..width).rev() {
        chars[idx] = BASE36_ALPHABET[(value % 36) as usize] as char;
        value /= 36;
    }
    chars.into_iter().collect()
}

fn generate_compact_run_id(now: i64) -> Result<String, OrchestratorError> {
    let timestamp = u64::try_from(now).map_err(|_| {
        OrchestratorError::SelectorValidation(
            "workflow_start requires a non-negative timestamp".to_string(),
        )
    })?;
    let mut bytes = [0_u8; 4];
    getrandom(&mut bytes).map_err(|err| {
        OrchestratorError::SelectorValidation(format!(
            "workflow_start failed to generate run id randomness: {err}"
        ))
    })?;
    let sample = u32::from_le_bytes(bytes) % RUN_SUFFIX_SPACE;
    let ts = base36_encode_u64(timestamp);
    let suffix = base36_encode_fixed_u32(sample, 4);
    Ok(format!("run-{ts}-{suffix}"))
}

fn run_metadata_path(state_root: &Path, run_id: &str) -> std::path::PathBuf {
    state_root
        .join("workflows/runs")
        .join(format!("{run_id}.json"))
}

fn allocate_compact_run_id_with_retry<F>(
    run_store: &WorkflowRunStore,
    now: i64,
    mut next_run_id: F,
) -> Result<String, OrchestratorError>
where
    F: FnMut(i64) -> Result<String, OrchestratorError>,
{
    for _ in 0..RUN_ID_MAX_GENERATION_ATTEMPTS {
        let run_id = next_run_id(now)?;
        if !run_metadata_path(run_store.state_root(), &run_id).exists() {
            return Ok(run_id);
        }
    }
    Err(OrchestratorError::SelectorValidation(format!(
        "failed to allocate unique workflow run id after {} attempts",
        RUN_ID_MAX_GENERATION_ATTEMPTS
    )))
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
    workflow_inputs: Option<&Map<String, Value>>,
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
    if let Some(thread_context) = request.thread_context.as_ref() {
        inputs.insert(
            "thread_context".to_string(),
            Value::String(thread_context.clone()),
        );
    }
    if let Some(source_message_id) = source_message_id {
        inputs.insert(
            "source_message_id".to_string(),
            Value::String(source_message_id.to_string()),
        );
    }
    if let Some(workflow_inputs) = workflow_inputs {
        inputs.insert(
            "workflow_inputs".to_string(),
            Value::Object(workflow_inputs.clone()),
        );
    }
    inputs
}

fn selector_start_memory_context(request: &SelectorRequest) -> RunMemoryContext {
    RunMemoryContext::from_selector_request(
        request.memory_bulletin.as_deref(),
        &request.memory_bulletin_citations,
    )
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
    CommandInvoke {
        result: Value,
    },
    NoResponse {
        reason: String,
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
    pub memory_enabled: bool,
    pub source_message_id: Option<&'a str>,
    pub workflow_inputs: Option<&'a Map<String, Value>>,
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

            let run_id = allocate_compact_run_id_with_retry(
                ctx.run_store,
                ctx.now,
                generate_compact_run_id,
            )?;
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
                    channel_profile_id: Some(request.channel_profile_id.clone()),
                    status_conversation_id: request.conversation_id.clone(),
                    memory_context: selector_start_memory_context(request),
                },
                selector_start_inputs(request, ctx.source_message_id, ctx.workflow_inputs),
                ctx.now,
            )?;
            let mut engine = WorkflowEngine::new(ctx.run_store.clone(), ctx.orchestrator.clone());
            if let Some(context) = ctx.workspace_access_context.clone() {
                engine = engine.with_workspace_access_context(context);
            }
            if let Some(binaries) = ctx.runner_binaries.clone() {
                engine = engine.with_runner_binaries(binaries);
            }
            engine = engine.with_memory_enabled(ctx.memory_enabled);
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
        SelectorAction::CommandInvoke => {
            let function_id = validated.function_id.ok_or_else(|| {
                OrchestratorError::SelectorValidation("missing functionId".to_string())
            })?;
            let function_args = validated.function_args.unwrap_or_default();
            let call = FunctionCall {
                function_id,
                args: function_args,
            };
            let invoke_result = ctx
                .functions
                .invoke_with_context(&call, Some(&ctx.orchestrator.id))?;
            Ok(RoutedSelectorAction::CommandInvoke {
                result: invoke_result,
            })
        }
        SelectorAction::NoResponse => Ok(RoutedSelectorAction::NoResponse {
            reason: validated
                .reason
                .unwrap_or_else(|| "selector chose no_response".to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn seed_run_metadata(state_root: &Path, run_id: &str) {
        let path = state_root
            .join("workflows/runs")
            .join(format!("{run_id}.json"));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create run metadata parent");
        }
        fs::write(path, b"{}").expect("seed run metadata");
    }

    #[test]
    fn allocates_compact_run_id_by_retrying_on_collision() {
        let dir = tempdir().expect("tempdir");
        let state_root = dir.path();
        let run_store = WorkflowRunStore::new(state_root);
        seed_run_metadata(state_root, "run-collision");

        let mut attempts = 0usize;
        let run_id = allocate_compact_run_id_with_retry(&run_store, 1, |_| {
            attempts += 1;
            Ok(if attempts == 1 {
                "run-collision".to_string()
            } else {
                "run-unique".to_string()
            })
        })
        .expect("allocate run id");

        assert_eq!(run_id, "run-unique");
        assert_eq!(attempts, 2);
    }

    #[test]
    fn allocate_compact_run_id_with_retry_fails_after_max_attempts() {
        let dir = tempdir().expect("tempdir");
        let state_root = dir.path();
        let run_store = WorkflowRunStore::new(state_root);
        seed_run_metadata(state_root, "run-collision");

        let err =
            allocate_compact_run_id_with_retry(&run_store, 1, |_| Ok("run-collision".to_string()))
                .expect_err("collisions should exhaust retries");

        assert!(
            err.to_string()
                .contains("failed to allocate unique workflow run id"),
            "unexpected error: {err}"
        );
    }
}
