use crate::config::OrchestratorConfig;
use crate::orchestration::diagnostics::append_security_log;
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::function_registry::{FunctionCall, FunctionRegistry};
use crate::orchestration::output_contract::resolve_step_output_paths;
use crate::orchestration::routing::{resolve_status_run_id, StatusResolutionInput};
use crate::orchestration::run_store::{
    ProgressSnapshot, SelectorStartedRunMetadata, WorkflowRunStore,
};
use crate::orchestration::selector::{
    parse_and_validate_selector_result, SelectorAction, SelectorRequest, SelectorResult,
};
use crate::orchestration::workflow_engine::WorkflowEngine;
use crate::orchestration::workspace_access::WorkspaceAccessContext;
use crate::provider::RunnerBinaries;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

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
            let diagnostics_root = ctx.run_store.state_root().join("orchestrator/artifacts");
            fs::create_dir_all(&diagnostics_root).map_err(|e| io_error(&diagnostics_root, e))?;

            let (findings, context_bundle) = if let Some(run_id_value) = run_id.clone() {
                match ctx.run_store.load_progress(&run_id_value) {
                    Ok(progress) => (
                        format!(
                            "Diagnostics summary for run {}: state={}, summary={}.",
                            run_id_value, progress.state, progress.summary
                        ),
                        Value::Object(Map::from_iter([
                            (
                                "diagnosticsId".to_string(),
                                Value::String(diagnostics_id.clone()),
                            ),
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
                            (
                                "diagnosticsId".to_string(),
                                Value::String(diagnostics_id.clone()),
                            ),
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

            let context_path =
                diagnostics_root.join(format!("diagnostics-context-{diagnostics_id}.json"));
            let result_path =
                diagnostics_root.join(format!("diagnostics-result-{diagnostics_id}.json"));

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
