use crate::config::OrchestratorConfig;
use crate::memory::{
    embed_query_text, hybrid_recall, persist_diagnostics_findings, DiagnosticsFindingWriteback,
    HybridRecallRequest, MemoryPaths, MemoryRecallOptions, MemoryRepository,
};
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

const DIAGNOSTICS_MEMORY_TOP_N: usize = 8;

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

fn diagnostics_memory_evidence_path(
    diagnostics_root: &Path,
    diagnostics_id: &str,
) -> std::path::PathBuf {
    diagnostics_root.join(format!("diagnostics-memory-evidence-{diagnostics_id}.json"))
}

fn persist_diagnostics_memory_evidence(
    diagnostics_root: &Path,
    diagnostics_id: &str,
    payload: &Value,
) -> Result<(), OrchestratorError> {
    let evidence_path = diagnostics_memory_evidence_path(diagnostics_root, diagnostics_id);
    fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(payload).map_err(|e| json_error(&evidence_path, e))?,
    )
    .map_err(|e| io_error(&evidence_path, e))?;
    Ok(())
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
    if let Some(source_message_id) = source_message_id {
        inputs.insert(
            "source_message_id".to_string(),
            Value::String(source_message_id.to_string()),
        );
    }
    if let Some(memory_bulletin) = request.memory_bulletin.as_ref() {
        inputs.insert(
            "memory_bulletin".to_string(),
            Value::String(memory_bulletin.clone()),
        );
    }
    if !request.memory_bulletin_citations.is_empty() {
        inputs.insert(
            "memory_bulletin_citations".to_string(),
            Value::Array(
                request
                    .memory_bulletin_citations
                    .iter()
                    .map(|value| Value::String(value.clone()))
                    .collect(),
            ),
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

            let (mut findings, context_bundle) = if let Some(run_id_value) = run_id.clone() {
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
            let mut context_bundle_map = match context_bundle {
                Value::Object(map) => map,
                _ => Map::new(),
            };

            let mut diagnostics_related_memory_ids: Vec<String> = Vec::new();
            if ctx.memory_enabled && run_id.is_some() {
                let memory_paths = MemoryPaths::from_runtime_root(ctx.run_store.state_root());
                match MemoryRepository::open(&memory_paths.database, &ctx.orchestrator.id)
                    .and_then(|repo| repo.ensure_schema().map(|_| repo))
                {
                    Ok(repo) => {
                        let recall_request = HybridRecallRequest {
                            requesting_orchestrator_id: ctx.orchestrator.id.clone(),
                            conversation_id: request.conversation_id.clone(),
                            query_text: request.user_message.clone(),
                            query_embedding: embed_query_text(&request.user_message),
                        };
                        let recall_options = MemoryRecallOptions {
                            top_n: DIAGNOSTICS_MEMORY_TOP_N,
                            ..MemoryRecallOptions::default()
                        };
                        match hybrid_recall(
                            &repo,
                            &recall_request,
                            &recall_options,
                            ctx.workspace_access_context.as_ref(),
                            &memory_paths.log_file,
                        ) {
                            Ok(recall) => {
                                let evidence_payload = Value::Object(Map::from_iter([
                                    (
                                        "diagnosticsId".to_string(),
                                        Value::String(diagnostics_id.clone()),
                                    ),
                                    (
                                        "mode".to_string(),
                                        Value::String(
                                            format!("{:?}", recall.mode).to_ascii_lowercase(),
                                        ),
                                    ),
                                    (
                                        "memories".to_string(),
                                        Value::Array(
                                            recall
                                                .memories
                                                .iter()
                                                .map(|entry| {
                                                    Value::Object(Map::from_iter([
                                                        (
                                                            "memoryId".to_string(),
                                                            Value::String(
                                                                entry.memory.memory_id.clone(),
                                                            ),
                                                        ),
                                                        (
                                                            "type".to_string(),
                                                            Value::String(format!(
                                                                "{:?}",
                                                                entry.memory.node_type
                                                            )),
                                                        ),
                                                        (
                                                            "summary".to_string(),
                                                            Value::String(
                                                                entry.memory.summary.clone(),
                                                            ),
                                                        ),
                                                        (
                                                            "importance".to_string(),
                                                            Value::from(entry.memory.importance),
                                                        ),
                                                        (
                                                            "confidence".to_string(),
                                                            Value::from(entry.memory.confidence),
                                                        ),
                                                        (
                                                            "sourceType".to_string(),
                                                            Value::String(
                                                                entry.citation.source_type.clone(),
                                                            ),
                                                        ),
                                                        (
                                                            "sourcePath".to_string(),
                                                            entry
                                                                .citation
                                                                .source_path
                                                                .as_ref()
                                                                .map(|p| {
                                                                    Value::String(
                                                                        p.display().to_string(),
                                                                    )
                                                                })
                                                                .unwrap_or(Value::Null),
                                                        ),
                                                        (
                                                            "conversationId".to_string(),
                                                            entry
                                                                .citation
                                                                .conversation_id
                                                                .as_ref()
                                                                .map(|v| Value::String(v.clone()))
                                                                .unwrap_or(Value::Null),
                                                        ),
                                                        (
                                                            "workflowRunId".to_string(),
                                                            entry
                                                                .citation
                                                                .workflow_run_id
                                                                .as_ref()
                                                                .map(|v| Value::String(v.clone()))
                                                                .unwrap_or(Value::Null),
                                                        ),
                                                        (
                                                            "stepId".to_string(),
                                                            entry
                                                                .citation
                                                                .step_id
                                                                .as_ref()
                                                                .map(|v| Value::String(v.clone()))
                                                                .unwrap_or(Value::Null),
                                                        ),
                                                        (
                                                            "finalScore".to_string(),
                                                            Value::from(entry.final_score),
                                                        ),
                                                        (
                                                            "unresolvedContradiction".to_string(),
                                                            Value::Bool(
                                                                entry.unresolved_contradiction,
                                                            ),
                                                        ),
                                                    ]))
                                                })
                                                .collect(),
                                        ),
                                    ),
                                    (
                                        "edges".to_string(),
                                        serde_json::to_value(&recall.edges).map_err(|e| {
                                            OrchestratorError::SelectorJson(e.to_string())
                                        })?,
                                    ),
                                ]));
                                context_bundle_map
                                    .insert("memoryEvidence".to_string(), evidence_payload.clone());
                                diagnostics_related_memory_ids = recall
                                    .memories
                                    .iter()
                                    .map(|entry| entry.memory.memory_id.clone())
                                    .collect();
                                findings.push_str(&format!(
                                    " Included {} memory evidence item(s).",
                                    recall.memories.len()
                                ));
                                persist_diagnostics_memory_evidence(
                                    &diagnostics_root,
                                    &diagnostics_id,
                                    &evidence_payload,
                                )?;
                            }
                            Err(err) => {
                                append_security_log(
                                    ctx.run_store.state_root(),
                                    &format!(
                                        "diagnostics memory evidence recall failed diagnostics_id={diagnostics_id}: {err}"
                                    ),
                                );
                                let failure_payload = Value::Object(Map::from_iter([
                                    (
                                        "diagnosticsId".to_string(),
                                        Value::String(diagnostics_id.clone()),
                                    ),
                                    ("status".to_string(), Value::String("failure".to_string())),
                                    (
                                        "failureKind".to_string(),
                                        Value::String("recall".to_string()),
                                    ),
                                    ("reason".to_string(), Value::String(err.to_string())),
                                    ("timestamp".to_string(), Value::from(ctx.now)),
                                ]));
                                context_bundle_map
                                    .insert("memoryEvidence".to_string(), failure_payload.clone());
                                persist_diagnostics_memory_evidence(
                                    &diagnostics_root,
                                    &diagnostics_id,
                                    &failure_payload,
                                )?;
                            }
                        }
                    }
                    Err(err) => {
                        append_security_log(
                            ctx.run_store.state_root(),
                            &format!(
                                "diagnostics memory repository unavailable diagnostics_id={diagnostics_id}: {err}"
                            ),
                        );
                        let failure_payload = Value::Object(Map::from_iter([
                            (
                                "diagnosticsId".to_string(),
                                Value::String(diagnostics_id.clone()),
                            ),
                            ("status".to_string(), Value::String("failure".to_string())),
                            (
                                "failureKind".to_string(),
                                Value::String("repository".to_string()),
                            ),
                            ("reason".to_string(), Value::String(err.to_string())),
                            ("timestamp".to_string(), Value::from(ctx.now)),
                        ]));
                        context_bundle_map
                            .insert("memoryEvidence".to_string(), failure_payload.clone());
                        persist_diagnostics_memory_evidence(
                            &diagnostics_root,
                            &diagnostics_id,
                            &failure_payload,
                        )?;
                    }
                }
            }

            let context_path =
                diagnostics_root.join(format!("diagnostics-context-{diagnostics_id}.json"));
            let result_path =
                diagnostics_root.join(format!("diagnostics-result-{diagnostics_id}.json"));

            fs::write(
                &context_path,
                serde_json::to_vec_pretty(&Value::Object(context_bundle_map))
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

            if ctx.memory_enabled {
                let memory_paths = MemoryPaths::from_runtime_root(ctx.run_store.state_root());
                match MemoryRepository::open(&memory_paths.database, &ctx.orchestrator.id)
                    .and_then(|repo| repo.ensure_schema().map(|_| repo))
                {
                    Ok(repo) => {
                        if let Err(err) = persist_diagnostics_findings(
                            &repo,
                            &DiagnosticsFindingWriteback {
                                orchestrator_id: &ctx.orchestrator.id,
                                diagnostics_id: &diagnostics_id,
                                run_id: run_id.as_deref(),
                                conversation_id: request.conversation_id.as_deref(),
                                findings: &findings,
                                related_memory_ids: &diagnostics_related_memory_ids,
                                captured_at: ctx.now,
                            },
                        ) {
                            append_security_log(
                                ctx.run_store.state_root(),
                                &format!(
                                    "diagnostics findings write-back failed diagnostics_id={diagnostics_id}: {err}"
                                ),
                            );
                        }
                    }
                    Err(err) => append_security_log(
                        ctx.run_store.state_root(),
                        &format!(
                            "diagnostics findings repository unavailable diagnostics_id={diagnostics_id}: {err}"
                        ),
                    ),
                }
            }

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
