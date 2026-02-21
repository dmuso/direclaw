use crate::config::{
    OrchestratorConfig, WorkflowConfig, WorkflowStepConfig, WorkflowStepWorkspaceMode,
};
use crate::memory::{
    persist_workflow_output_memories, MemoryPaths, MemoryRepository, WorkflowOutputWriteback,
};
use crate::orchestration::diagnostics::{
    append_security_log, persist_provider_invocation_log, provider_error_log,
};
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::output_contract::{
    evaluate_step_result, materialize_output_files, resolve_step_output_paths, StepEvaluation,
};
use crate::orchestration::prompt_render::render_step_prompt;
use crate::orchestration::run_store::{StepAttemptRecord, WorkflowRunRecord, WorkflowRunStore};
use crate::orchestration::workspace_access::{
    enforce_workspace_access, resolve_agent_workspace_root, WorkspaceAccessContext,
};
use crate::prompts::{
    context_path_for_prompt_reference, default_step_context, is_prompt_template_reference,
    resolve_prompt_template_path, PROMPTS_DIR,
};
use crate::provider::{
    consume_reset_flag, run_provider, write_file_backed_prompt, PromptArtifacts, ProviderError,
    ProviderKind, ProviderRequest, RunnerBinaries,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

pub(crate) struct StepExecutionContext<'a> {
    pub run_store: &'a WorkflowRunStore,
    pub orchestrator: &'a OrchestratorConfig,
    pub workspace_access_context: Option<&'a WorkspaceAccessContext>,
    pub runner_binaries: &'a RunnerBinaries,
    pub step_timeout_seconds: u64,
    pub memory_enabled: bool,
}

pub(crate) fn execute_step_attempt(
    context: &StepExecutionContext<'_>,
    run: &WorkflowRunRecord,
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
    attempt: u32,
    now: i64,
) -> Result<StepEvaluation, OrchestratorError> {
    let orchestrator_workspace = if let Some(workspace) = context.workspace_access_context {
        workspace.private_workspace_root.clone()
    } else {
        context.run_store.state_root().to_path_buf()
    };
    let run_workspace = if let Some(workspace) = context.workspace_access_context {
        workspace
            .private_workspace_root
            .join("workflows/runs")
            .join(&run.run_id)
            .join("workspace")
    } else {
        context
            .run_store
            .state_root()
            .join("workflows/runs")
            .join(&run.run_id)
            .join("workspace")
    };
    let prompt_root = if let Some(workspace) = context.workspace_access_context {
        workspace.private_workspace_root.join(PROMPTS_DIR)
    } else {
        context.run_store.state_root().join(PROMPTS_DIR)
    };

    let agent = context
        .orchestrator
        .agents
        .get(&step.agent)
        .ok_or_else(|| OrchestratorError::StepExecution {
            step_id: step.id.clone(),
            reason: format!("step references unknown agent `{}`", step.agent),
        })?;
    let agent_workspace = resolve_agent_workspace_root(
        context
            .workspace_access_context
            .map(|ctx| ctx.private_workspace_root.as_path())
            .unwrap_or_else(|| context.run_store.state_root()),
        &step.agent,
        agent,
    );
    let execution_cwd = match step.workspace_mode {
        WorkflowStepWorkspaceMode::OrchestratorWorkspace => orchestrator_workspace.clone(),
        WorkflowStepWorkspaceMode::RunWorkspace => run_workspace.clone(),
        WorkflowStepWorkspaceMode::AgentWorkspace => agent_workspace.clone(),
    };

    if let Some(workspace) = context.workspace_access_context {
        if let Err(err) = enforce_workspace_access(
            workspace,
            &[
                orchestrator_workspace.clone(),
                run_workspace.clone(),
                agent_workspace.clone(),
                execution_cwd.clone(),
            ],
        ) {
            append_security_log(
                context.run_store.state_root(),
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

    let step_outputs = load_latest_step_outputs(
        context.run_store.state_root(),
        &run.run_id,
        workflow,
        &step.id,
    )?;
    let output_paths =
        match resolve_step_output_paths(context.run_store.state_root(), &run.run_id, step, attempt)
        {
            Ok(paths) => paths,
            Err(err @ OrchestratorError::OutputPathValidation { .. }) => {
                append_security_log(
                    context.run_store.state_root(),
                    &format!(
                        "output path validation denied for run `{}` step `{}` attempt `{}`: {}",
                        run.run_id, step.id, attempt, err
                    ),
                );
                return Err(err);
            }
            Err(err) => return Err(err),
        };
    let (prompt_template, context_template) = load_step_templates(&prompt_root, workflow, step)
        .map_err(|reason| OrchestratorError::StepExecution {
            step_id: step.id.clone(),
            reason,
        })?;
    let rendered = render_step_prompt(
        run,
        workflow,
        step,
        attempt,
        &run_workspace,
        &output_paths,
        &step_outputs,
        &prompt_template,
        &context_template,
    )?;

    let attempt_dir = context
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
        timeout: Duration::from_secs(context.step_timeout_seconds),
        reset_requested: reset_resolution.reset_requested,
        fresh_on_failure: false,
        env_overrides: BTreeMap::new(),
    };

    let provider_output =
        run_provider(&provider_request, context.runner_binaries).map_err(|err| {
            if let Some(log) = provider_error_log(&err) {
                let _ = persist_provider_invocation_log(&attempt_dir, log);
            }
            match err {
                ProviderError::Timeout { .. } => OrchestratorError::StepTimeout {
                    step_timeout_seconds: context.step_timeout_seconds,
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
    evaluation.output_files = materialize_output_files(step, &evaluation.outputs, &output_paths)?;
    if context.memory_enabled {
        let memory_paths = MemoryPaths::from_runtime_root(context.run_store.state_root());
        match MemoryRepository::open(&memory_paths.database, &context.orchestrator.id)
            .and_then(|repo| repo.ensure_schema().map(|_| repo))
        {
            Ok(repo) => {
                if let Err(err) = persist_workflow_output_memories(
                    &repo,
                    &WorkflowOutputWriteback {
                        orchestrator_id: &context.orchestrator.id,
                        run_id: &run.run_id,
                        step_id: &step.id,
                        attempt,
                        conversation_id: run.inputs.get("conversation_id").and_then(Value::as_str),
                        outputs: &evaluation.outputs,
                        output_files: &evaluation.output_files,
                        captured_at: now,
                    },
                ) {
                    append_security_log(
                        context.run_store.state_root(),
                        &format!(
                            "memory output write-back failed run_id={} step_id={} attempt={attempt}: {err}",
                            run.run_id, step.id
                        ),
                    );
                }
            }
            Err(err) => append_security_log(
                context.run_store.state_root(),
                &format!(
                    "memory repository unavailable for run_id={} step_id={} attempt={attempt}: {err}",
                    run.run_id, step.id
                ),
            ),
        }
    }
    context.run_store.append_engine_log(
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

fn load_step_templates(
    prompt_root: &Path,
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
) -> Result<(String, String), String> {
    if !is_prompt_template_reference(&step.prompt) {
        return Ok((step.prompt.clone(), default_step_context().to_string()));
    }
    let prompt_path = resolve_prompt_template_path(prompt_root, step.prompt.trim())?;
    let context_rel = context_path_for_prompt_reference(step.prompt.trim());
    let context_path = resolve_prompt_template_path(prompt_root, &context_rel)?;

    let prompt_template = fs::read_to_string(&prompt_path).map_err(|err| {
        format!(
            "failed to read prompt template for workflow `{}` step `{}` at {}: {err}",
            workflow.id,
            step.id,
            prompt_path.display()
        )
    })?;
    let context_template = fs::read_to_string(&context_path).map_err(|err| {
        format!(
            "failed to read context template for workflow `{}` step `{}` at {}: {err}",
            workflow.id,
            step.id,
            context_path.display()
        )
    })?;
    Ok((prompt_template, context_template))
}

pub fn resolve_runner_binaries() -> RunnerBinaries {
    RunnerBinaries {
        anthropic: std::env::var("DIRECLAW_PROVIDER_BIN_ANTHROPIC")
            .unwrap_or_else(|_| "claude".to_string()),
        openai: std::env::var("DIRECLAW_PROVIDER_BIN_OPENAI")
            .unwrap_or_else(|_| "codex".to_string()),
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

fn provider_instruction_message(artifacts: &PromptArtifacts) -> String {
    let prompt_parent = artifacts.prompt_file.parent();
    let shared_parent = prompt_parent.is_some()
        && artifacts
            .context_files
            .iter()
            .all(|path| path.parent() == prompt_parent);

    if shared_parent {
        let mut file_names = Vec::with_capacity(artifacts.context_files.len() + 1);
        file_names.push(
            artifacts
                .prompt_file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("prompt.md")
                .to_string(),
        );
        file_names.extend(artifacts.context_files.iter().map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("context.md")
                .to_string()
        }));
        return format!(
            "Read file(s) {} from {}. Execute exactly as instructed in those files.",
            file_names.join(", "),
            prompt_parent.unwrap_or_else(|| Path::new(".")).display()
        );
    }

    let mut paths = Vec::with_capacity(artifacts.context_files.len() + 1);
    paths.push(artifacts.prompt_file.display().to_string());
    paths.extend(
        artifacts
            .context_files
            .iter()
            .map(|path| path.display().to_string()),
    );
    format!(
        "Read file(s) {}. Execute exactly as instructed in those files.",
        paths.join(", ")
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

#[cfg(test)]
mod tests {
    use super::provider_instruction_message;
    use crate::provider::PromptArtifacts;
    use std::path::PathBuf;

    #[test]
    fn provider_instruction_message_uses_shared_parent_once() {
        let artifacts = PromptArtifacts {
            prompt_file: PathBuf::from(
                "/tmp/.direclaw/workflows/runs/run-abc/steps/plan/attempts/1/prompt.md",
            ),
            context_files: vec![PathBuf::from(
                "/tmp/.direclaw/workflows/runs/run-abc/steps/plan/attempts/1/context.md",
            )],
        };

        let message = provider_instruction_message(&artifacts);

        assert!(message.contains("Read file(s) prompt.md, context.md from"));
        assert!(message.contains("/tmp/.direclaw/workflows/runs/run-abc/steps/plan/attempts/1"));
        assert!(!message.contains("prompt file at"));
    }

    #[test]
    fn provider_instruction_message_falls_back_to_explicit_paths_when_roots_differ() {
        let artifacts = PromptArtifacts {
            prompt_file: PathBuf::from("/tmp/a/prompt.md"),
            context_files: vec![PathBuf::from("/tmp/b/context.md")],
        };

        let message = provider_instruction_message(&artifacts);

        assert!(message.contains("/tmp/a/prompt.md, /tmp/b/context.md"));
        assert!(!message.contains("from /tmp/a"));
    }
}
