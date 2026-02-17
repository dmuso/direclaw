use crate::app::command_support::{
    load_orchestrator_or_err, load_settings, now_nanos, now_secs, save_orchestrator_config,
};
use crate::config::{
    normalize_workflow_input_key, WorkflowConfig, WorkflowId, WorkflowInputs, WorkflowStepConfig,
    WorkflowStepPromptType, WorkflowStepType, WorkflowStepWorkspaceMode, WorkflowTag,
};
use crate::orchestration::run_store::{RunState, WorkflowRunStore};
use crate::orchestration::workflow_engine::WorkflowEngine;
use crate::orchestration::workspace_access::verify_orchestrator_workspace_access;
use crate::templates::workflow_step_defaults::{
    default_step_output_contract, default_step_output_files, default_step_output_priority,
    default_step_scaffold,
};
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

pub fn cmd_workflow(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err(
            "usage: workflow <list|show|add|remove|run|status|progress|cancel> ...".to_string(),
        );
    }

    match args[0].as_str() {
        "list" => {
            if args.len() != 2 {
                return Err("usage: workflow list <orchestrator_id>".to_string());
            }
            let settings = load_settings()?;
            let orchestrator = load_orchestrator_or_err(&settings, &args[1])?;
            Ok(orchestrator
                .workflows
                .iter()
                .map(|w| w.id.clone())
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "show" => {
            if args.len() != 3 {
                return Err("usage: workflow show <orchestrator_id> <workflow_id>".to_string());
            }
            let settings = load_settings()?;
            let orchestrator = load_orchestrator_or_err(&settings, &args[1])?;
            let workflow = orchestrator
                .workflows
                .iter()
                .find(|w| w.id == args[2])
                .ok_or_else(|| format!("invalid workflow id `{}`", args[2]))?;
            serde_yaml::to_string(workflow).map_err(|e| format!("failed to encode workflow: {e}"))
        }
        "add" => {
            if args.len() != 3 {
                return Err("usage: workflow add <orchestrator_id> <workflow_id>".to_string());
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let workflow_id = args[2].clone();
            WorkflowId::parse(&workflow_id)?;
            let mut orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            if orchestrator.workflows.iter().any(|w| w.id == workflow_id) {
                return Err(format!("workflow `{workflow_id}` already exists"));
            }
            let selector = orchestrator.selector_agent.clone();
            orchestrator.workflows.push(WorkflowConfig {
                id: workflow_id.clone(),
                version: 1,
                description: format!("{workflow_id} workflow"),
                tags: vec![WorkflowTag::parse(&workflow_id)?],
                inputs: WorkflowInputs::default(),
                limits: None,
                steps: vec![WorkflowStepConfig {
                    id: "step_1".to_string(),
                    step_type: WorkflowStepType::AgentTask,
                    agent: selector,
                    prompt: default_step_scaffold("agent_task"),
                    prompt_type: WorkflowStepPromptType::FileOutput,
                    workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                    next: None,
                    on_approve: None,
                    on_reject: None,
                    outputs: default_step_output_contract("agent_task"),
                    output_files: default_step_output_files("agent_task"),
                    final_output_priority: default_step_output_priority("agent_task"),
                    limits: None,
                }],
            });
            save_orchestrator_config(&settings, orchestrator_id, &orchestrator)?;
            Ok(format!(
                "workflow added\norchestrator={}\nworkflow={}",
                orchestrator_id, workflow_id
            ))
        }
        "remove" => {
            if args.len() != 3 {
                return Err("usage: workflow remove <orchestrator_id> <workflow_id>".to_string());
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let workflow_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            if orchestrator.default_workflow == workflow_id {
                return Err("cannot remove default workflow".to_string());
            }
            let before = orchestrator.workflows.len();
            orchestrator.workflows.retain(|w| w.id != workflow_id);
            if orchestrator.workflows.len() == before {
                return Err(format!("invalid workflow id `{}`", args[2]));
            }
            save_orchestrator_config(&settings, orchestrator_id, &orchestrator)?;
            Ok(format!(
                "workflow removed\norchestrator={}\nworkflow={}",
                orchestrator_id, workflow_id
            ))
        }
        "run" => {
            if args.len() < 3 {
                return Err(
                    "usage: workflow run <orchestrator_id> <workflow_id> [--input key=value ...]"
                        .to_string(),
                );
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let workflow_id = &args[2];
            let orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            let workspace_context =
                verify_orchestrator_workspace_access(&settings, orchestrator_id, &orchestrator)
                    .map_err(|e| e.to_string())?;
            if !orchestrator.workflows.iter().any(|w| &w.id == workflow_id) {
                return Err(format!("invalid workflow id `{workflow_id}`"));
            }
            let selector = orchestrator
                .agents
                .get(&orchestrator.selector_agent)
                .ok_or_else(|| "selector agent is missing".to_string())?;
            if !selector.can_orchestrate_workflows {
                return Err("selector agent cannot orchestrate workflows".to_string());
            }

            let input_map = parse_key_value_inputs(&args[3..])?;
            let runtime_root = settings
                .resolve_orchestrator_runtime_root(orchestrator_id)
                .map_err(|e| e.to_string())?;
            fs::create_dir_all(&runtime_root)
                .map_err(|e| format!("failed to create {}: {e}", runtime_root.display()))?;
            let store = WorkflowRunStore::new(&runtime_root);
            let run_id = format!("run-{}-{}-{}", orchestrator_id, workflow_id, now_nanos());
            store
                .create_run_with_inputs(run_id.clone(), workflow_id.clone(), input_map, now_secs())
                .map_err(|e| e.to_string())?;
            let engine = WorkflowEngine::new(store.clone(), orchestrator.clone())
                .with_workspace_access_context(workspace_context);
            engine
                .start(&run_id, now_secs())
                .map_err(|e| e.to_string())?;
            Ok(format!("workflow started\nrun_id={run_id}"))
        }
        "status" => {
            if args.len() != 2 {
                return Err("usage: workflow status <run_id>".to_string());
            }
            let settings = load_settings()?;
            let store = run_store_for_run_id(&settings, &args[1])?;
            let run = store.load_run(&args[1]).map_err(|e| e.to_string())?;
            let progress = store.load_progress(&args[1]).map_err(|e| e.to_string())?;
            let mut input_keys = run.inputs.keys().cloned().collect::<Vec<_>>();
            input_keys.sort();
            Ok(format!(
                "run_id={}\nstate={}\nsummary={}\ninput_count={}\ninput_keys={}",
                progress.run_id,
                progress.state,
                progress.summary,
                run.inputs.len(),
                input_keys.join(",")
            ))
        }
        "progress" => {
            if args.len() != 2 {
                return Err("usage: workflow progress <run_id>".to_string());
            }
            let settings = load_settings()?;
            let store = run_store_for_run_id(&settings, &args[1])?;
            let progress = store.load_progress(&args[1]).map_err(|e| e.to_string())?;
            serde_json::to_string_pretty(&progress)
                .map_err(|e| format!("failed to encode workflow progress: {e}"))
        }
        "cancel" => {
            if args.len() != 2 {
                return Err("usage: workflow cancel <run_id>".to_string());
            }
            let settings = load_settings()?;
            let store = run_store_for_run_id(&settings, &args[1])?;
            let mut run = store.load_run(&args[1]).map_err(|e| e.to_string())?;
            if !run.state.clone().is_terminal() {
                store
                    .transition_state(
                        &mut run,
                        RunState::Canceled,
                        now_secs(),
                        "canceled by command",
                        false,
                        "none",
                    )
                    .map_err(|e| e.to_string())?;
            }
            Ok(format!(
                "workflow canceled\nrun_id={}\nstate={}",
                run.run_id, run.state
            ))
        }
        other => Err(format!("unknown workflow subcommand `{other}`")),
    }
}

fn parse_key_value_inputs(args: &[String]) -> Result<Map<String, Value>, String> {
    if args.is_empty() {
        return Ok(Map::new());
    }

    let mut map = Map::new();
    let mut i = 0usize;
    while i < args.len() {
        if args[i] != "--input" {
            return Err(format!("unexpected argument `{}`", args[i]));
        }
        if i + 1 >= args.len() {
            return Err("--input requires key=value".to_string());
        }
        let raw = &args[i + 1];
        let (key, value) = raw
            .split_once('=')
            .ok_or_else(|| "--input requires key=value".to_string())?;
        let normalized = normalize_workflow_input_key(key)?;
        map.insert(normalized, Value::String(value.to_string()));
        i += 2;
    }

    Ok(map)
}

fn run_store_for_run_id(
    settings: &crate::config::Settings,
    run_id: &str,
) -> Result<WorkflowRunStore, String> {
    for orchestrator_id in settings.orchestrators.keys() {
        let runtime_root = settings
            .resolve_orchestrator_runtime_root(orchestrator_id)
            .map_err(|e| e.to_string())?;
        if run_record_exists(&runtime_root, run_id) {
            return Ok(WorkflowRunStore::new(runtime_root));
        }
    }
    Err(format!("unknown workflow run `{run_id}`"))
}

fn run_record_exists(runtime_root: &Path, run_id: &str) -> bool {
    runtime_root
        .join("workflows/runs")
        .join(format!("{run_id}.json"))
        .is_file()
}
