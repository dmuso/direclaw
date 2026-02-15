use crate::config::{
    OrchestratorConfig, WorkflowConfig, WorkflowStepConfig, WorkflowStepWorkspaceMode,
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
    let rendered = render_step_prompt(
        run,
        workflow,
        step,
        attempt,
        &run_workspace,
        &output_paths,
        &step_outputs,
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
