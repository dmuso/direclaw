use crate::app::command_handlers::attach::cmd_attach;
use crate::app::command_handlers::auth::{cmd_auth, render_auth_sync_result, sync_auth_sources};
use crate::app::command_handlers::doctor::cmd_doctor;
use crate::app::command_handlers::provider::{cmd_model, cmd_provider};
use crate::app::command_handlers::update::cmd_update;
use crate::config::{
    default_global_config_path, load_orchestrator_config, normalize_workflow_input_key,
    AgentConfig, ChannelKind, ChannelProfile, ConfigError, ConfigProviderKind, OrchestratorConfig,
    Settings, SettingsOrchestrator, ValidationOptions, WorkflowConfig, WorkflowInputs,
    WorkflowStepConfig, WorkflowStepPromptType, WorkflowStepType, WorkflowStepWorkspaceMode,
};
use crate::orchestrator::{
    verify_orchestrator_workspace_access, OrchestratorError, RunState, WorkflowEngine,
    WorkflowRunStore,
};
use crate::queue::IncomingMessage;
use crate::runtime::{
    append_runtime_log, bootstrap_state_root, cleanup_stale_supervisor, default_state_root_path,
    load_supervisor_state, reserve_start_lock, run_supervisor, save_supervisor_state,
    spawn_supervisor_process, stop_active_supervisor, supervisor_ownership_state, OwnershipState,
    StatePaths, SupervisorState, WorkerHealth, WorkerState,
};
use crate::slack;
use crate::workflow::{
    default_step_output_contract, default_step_output_files, default_step_scaffold,
    initial_orchestrator_config, WorkflowTemplate,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

pub use crate::app::cli::cli_help_lines;
pub use crate::app::cli::parse_cli_verb;
pub use crate::app::cli::selector_help_lines;
pub use crate::app::cli::CliVerb;
pub use crate::app::command_catalog::function_ids;
pub use crate::app::command_catalog::FunctionArgDef;
pub use crate::app::command_catalog::FunctionArgTypeDef;
pub use crate::app::command_catalog::FunctionDef;
pub use crate::app::command_catalog::V1_FUNCTIONS;
pub use crate::app::command_dispatch::plan_function_invocation;
pub use crate::app::command_dispatch::FunctionExecutionPlan;
pub use crate::app::command_dispatch::InternalFunction;

#[derive(Debug, Clone, Copy)]
pub struct FunctionExecutionContext<'a> {
    pub run_store: Option<&'a WorkflowRunStore>,
    pub settings: Option<&'a Settings>,
}

pub fn execute_function_invocation(
    function_id: &str,
    args: &Map<String, Value>,
    context: FunctionExecutionContext<'_>,
) -> Result<Value, OrchestratorError> {
    match plan_function_invocation(function_id, args)
        .map_err(OrchestratorError::SelectorValidation)?
    {
        FunctionExecutionPlan::CliArgs(cli_args) => execute_cli_plan(cli_args),
        FunctionExecutionPlan::Internal(internal) => execute_internal_function(internal, context),
    }
}

fn execute_cli_plan(cli_args: Vec<String>) -> Result<Value, OrchestratorError> {
    let command = cli_args.join(" ");
    let output = run_cli(cli_args).map_err(OrchestratorError::SelectorValidation)?;
    Ok(Value::Object(Map::from_iter([
        ("command".to_string(), Value::String(command)),
        ("output".to_string(), Value::String(output)),
    ])))
}

pub fn execute_internal_function(
    command: InternalFunction,
    context: FunctionExecutionContext<'_>,
) -> Result<Value, OrchestratorError> {
    match command {
        InternalFunction::WorkflowList { orchestrator_id } => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow.list requires settings context".to_string(),
                )
            })?;
            let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
            Ok(Value::Object(Map::from_iter([
                ("orchestratorId".to_string(), Value::String(orchestrator_id)),
                (
                    "workflows".to_string(),
                    Value::Array(
                        orchestrator
                            .workflows
                            .iter()
                            .map(|workflow| Value::String(workflow.id.clone()))
                            .collect(),
                    ),
                ),
            ])))
        }
        InternalFunction::WorkflowShow {
            orchestrator_id,
            workflow_id,
        } => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow.show requires settings context".to_string(),
                )
            })?;
            let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
            let workflow = orchestrator
                .workflows
                .iter()
                .find(|workflow| workflow.id == workflow_id)
                .ok_or_else(|| {
                    OrchestratorError::SelectorValidation(format!(
                        "workflow `{workflow_id}` not found in orchestrator `{orchestrator_id}`"
                    ))
                })?;
            Ok(Value::Object(Map::from_iter([
                ("orchestratorId".to_string(), Value::String(orchestrator_id)),
                ("workflowId".to_string(), Value::String(workflow_id)),
                (
                    "workflow".to_string(),
                    serde_json::to_value(workflow)
                        .map_err(|error| OrchestratorError::SelectorJson(error.to_string()))?,
                ),
            ])))
        }
        InternalFunction::WorkflowStatus { run_id }
        | InternalFunction::WorkflowProgress { run_id } => {
            let run_store = context.run_store.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow.status/progress requires workflow run store".to_string(),
                )
            })?;
            let progress = run_store
                .load_progress(&run_id)
                .map_err(|error| remap_missing_run_error(&run_id, error))?;
            Ok(Value::Object(Map::from_iter([
                ("runId".to_string(), Value::String(run_id)),
                (
                    "progress".to_string(),
                    serde_json::to_value(progress)
                        .map_err(|error| OrchestratorError::SelectorJson(error.to_string()))?,
                ),
            ])))
        }
        InternalFunction::WorkflowCancel { run_id } => {
            let run_store = context.run_store.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow.cancel requires workflow run store".to_string(),
                )
            })?;
            let mut run = run_store
                .load_run(&run_id)
                .map_err(|error| remap_missing_run_error(&run_id, error))?;
            if !run.state.clone().is_terminal() {
                let now = run.updated_at.saturating_add(1);
                run_store.transition_state(
                    &mut run,
                    RunState::Canceled,
                    now,
                    "canceled by command",
                    false,
                    "none",
                )?;
            }
            Ok(Value::Object(Map::from_iter([
                ("runId".to_string(), Value::String(run_id)),
                ("state".to_string(), Value::String(run.state.to_string())),
            ])))
        }
        InternalFunction::OrchestratorList => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "orchestrator.list requires settings context".to_string(),
                )
            })?;
            Ok(Value::Object(Map::from_iter([(
                "orchestrators".to_string(),
                Value::Array(
                    settings
                        .orchestrators
                        .keys()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            )])))
        }
        InternalFunction::OrchestratorShow { orchestrator_id } => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "orchestrator.show requires settings context".to_string(),
                )
            })?;
            let entry = settings
                .orchestrators
                .get(&orchestrator_id)
                .ok_or_else(|| {
                    OrchestratorError::SelectorValidation(format!(
                        "unknown orchestrator `{orchestrator_id}`"
                    ))
                })?;
            let private_workspace = settings.resolve_private_workspace(&orchestrator_id)?;
            Ok(Value::Object(Map::from_iter([
                ("orchestratorId".to_string(), Value::String(orchestrator_id)),
                (
                    "privateWorkspace".to_string(),
                    Value::String(private_workspace.display().to_string()),
                ),
                (
                    "sharedAccess".to_string(),
                    Value::Array(
                        entry
                            .shared_access
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                ),
            ])))
        }
        InternalFunction::ChannelProfileList => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "channel_profile.list requires settings context".to_string(),
                )
            })?;
            Ok(Value::Object(Map::from_iter([(
                "channelProfiles".to_string(),
                Value::Array(
                    settings
                        .channel_profiles
                        .keys()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            )])))
        }
        InternalFunction::ChannelProfileShow { channel_profile_id } => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "channel_profile.show requires settings context".to_string(),
                )
            })?;
            let profile = settings
                .channel_profiles
                .get(&channel_profile_id)
                .ok_or_else(|| {
                    OrchestratorError::SelectorValidation(format!(
                        "unknown channel profile `{channel_profile_id}`"
                    ))
                })?;
            Ok(Value::Object(Map::from_iter([
                (
                    "channelProfileId".to_string(),
                    Value::String(channel_profile_id),
                ),
                (
                    "channel".to_string(),
                    Value::String(profile.channel.to_string()),
                ),
                (
                    "orchestratorId".to_string(),
                    Value::String(profile.orchestrator_id.clone()),
                ),
                (
                    "slackAppUserId".to_string(),
                    profile
                        .slack_app_user_id
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "requireMentionInChannels".to_string(),
                    profile
                        .require_mention_in_channels
                        .map(Value::Bool)
                        .unwrap_or(Value::Null),
                ),
            ])))
        }
    }
}

fn remap_missing_run_error(run_id: &str, err: OrchestratorError) -> OrchestratorError {
    match err {
        OrchestratorError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            OrchestratorError::UnknownRunId {
                run_id: run_id.to_string(),
            }
        }
        _ => err,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RuntimePreferences {
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
}

pub fn run_cli(args: Vec<String>) -> Result<String, String> {
    if args.is_empty() {
        return Ok(crate::app::cli::help_text());
    }

    match parse_cli_verb(args[0].as_str()) {
        CliVerb::Setup => crate::tui::setup::cmd_setup(),
        CliVerb::Start => cmd_start(),
        CliVerb::Stop => cmd_stop(),
        CliVerb::Restart => cmd_restart(),
        CliVerb::Status => cmd_status(),
        CliVerb::Logs => cmd_logs(),
        CliVerb::Send => cmd_send(&args[1..]),
        CliVerb::Update => cmd_update(&args[1..]),
        CliVerb::Doctor => cmd_doctor(),
        CliVerb::Attach => cmd_attach(),
        CliVerb::Channels => cmd_channels(&args[1..]),
        CliVerb::Provider => cmd_provider(&args[1..]),
        CliVerb::Model => cmd_model(&args[1..]),
        CliVerb::Agent => cmd_orchestrator_agent(&args[1..]),
        CliVerb::Orchestrator => cmd_orchestrator(&args[1..]),
        CliVerb::OrchestratorAgent => cmd_orchestrator_agent(&args[1..]),
        CliVerb::Workflow => cmd_workflow(&args[1..]),
        CliVerb::ChannelProfile => cmd_channel_profile(&args[1..]),
        CliVerb::Auth => cmd_auth(&args[1..]),
        CliVerb::Supervisor => cmd_supervisor(&args[1..]),
        CliVerb::Unknown => Err(format!("unknown command `{}`", args[0])),
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

pub(crate) fn map_config_err(err: ConfigError) -> String {
    err.to_string()
}

fn state_root() -> Result<PathBuf, String> {
    default_state_root_path().map_err(|e| e.to_string())
}

pub(crate) fn ensure_runtime_root() -> Result<StatePaths, String> {
    let root = state_root()?;
    let paths = StatePaths::new(root);
    bootstrap_state_root(&paths).map_err(|e| e.to_string())?;
    Ok(paths)
}

fn preferences_path(paths: &StatePaths) -> PathBuf {
    paths.root.join("runtime/preferences.yaml")
}

pub(crate) fn load_preferences(paths: &StatePaths) -> Result<RuntimePreferences, String> {
    let path = preferences_path(paths);
    if !path.exists() {
        return Ok(RuntimePreferences::default());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_yaml::from_str(&raw).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

pub(crate) fn save_preferences(
    paths: &StatePaths,
    prefs: &RuntimePreferences,
) -> Result<(), String> {
    let path = preferences_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let body =
        serde_yaml::to_string(prefs).map_err(|e| format!("failed to encode preferences: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

pub(crate) fn load_settings() -> Result<Settings, String> {
    let path = default_global_config_path().map_err(map_config_err)?;
    let settings = Settings::from_path(&path).map_err(map_config_err)?;
    settings
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .map_err(map_config_err)?;
    Ok(settings)
}

pub(crate) fn save_settings(settings: &Settings) -> Result<PathBuf, String> {
    settings
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .map_err(map_config_err)?;

    let path = default_global_config_path().map_err(map_config_err)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let body =
        serde_yaml::to_string(settings).map_err(|e| format!("failed to encode settings: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(path)
}

fn save_orchestrator_config(
    settings: &Settings,
    orchestrator_id: &str,
    orchestrator: &OrchestratorConfig,
) -> Result<PathBuf, String> {
    orchestrator
        .validate(settings, orchestrator_id)
        .map_err(map_config_err)?;
    let private_workspace = settings
        .resolve_private_workspace(orchestrator_id)
        .map_err(map_config_err)?;
    fs::create_dir_all(&private_workspace)
        .map_err(|e| format!("failed to create {}: {e}", private_workspace.display()))?;
    let path = private_workspace.join("orchestrator.yaml");
    let body = serde_yaml::to_string(orchestrator)
        .map_err(|e| format!("failed to encode orchestrator: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(path)
}

pub(crate) fn save_orchestrator_registry(
    settings: &Settings,
    registry: &BTreeMap<String, OrchestratorConfig>,
) -> Result<PathBuf, String> {
    let mut saved = None;
    for (orchestrator_id, orchestrator) in registry {
        let path = save_orchestrator_config(settings, orchestrator_id, orchestrator)?;
        saved = Some(path);
    }
    saved.ok_or_else(|| "no orchestrator configs to save".to_string())
}

fn remove_orchestrator_config(settings: &Settings, orchestrator_id: &str) -> Result<(), String> {
    let private_workspace = settings
        .resolve_private_workspace(orchestrator_id)
        .map_err(map_config_err)?;
    let path = private_workspace.join("orchestrator.yaml");
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).map_err(|e| format!("failed to remove {}: {e}", path.display()))
}

fn load_orchestrator_or_err(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<OrchestratorConfig, String> {
    load_orchestrator_config(settings, orchestrator_id).map_err(map_config_err)
}

fn cmd_start() -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let settings = load_settings()?;
    let auth_sync = sync_auth_sources(&settings)?;
    match supervisor_ownership_state(&paths).map_err(|e| e.to_string())? {
        OwnershipState::Running { pid } => {
            return Err(format!("supervisor already running (pid={pid})"))
        }
        OwnershipState::Stale => cleanup_stale_supervisor(&paths).map_err(|e| e.to_string())?,
        OwnershipState::NotRunning => {}
    }

    reserve_start_lock(&paths).map_err(|e| e.to_string())?;
    let pid = match spawn_supervisor_process(&paths.root).and_then(|pid| {
        crate::runtime::write_supervisor_lock_pid(&paths, pid)?;
        Ok(pid)
    }) {
        Ok(pid) => pid,
        Err(err) => {
            crate::runtime::clear_start_lock(&paths);
            return Err(err.to_string());
        }
    };

    append_runtime_log(
        &paths,
        "info",
        "supervisor.start.requested",
        &format!("pid={pid}"),
    );

    Ok(format!(
        "started\nstate_root={}\npid={}\n{}",
        paths.root.display(),
        pid,
        render_auth_sync_result(&auth_sync, false)
    ))
}

fn cmd_stop() -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    match stop_active_supervisor(&paths, Duration::from_secs(5)) {
        Ok(result) => {
            if result.forced {
                Ok(format!("stopped\npid={}\nforced=true", result.pid))
            } else {
                Ok(format!("stopped\npid={}\nforced=false", result.pid))
            }
        }
        Err(crate::runtime::RuntimeError::NotRunning) => Ok("stopped\nrunning=false".to_string()),
        Err(err) => Err(err.to_string()),
    }
}

fn cmd_restart() -> Result<String, String> {
    let stop = cmd_stop()?;
    let start = cmd_start()?;
    Ok(format!("restart complete\n{stop}\n{start}"))
}

fn classify_slack_profile_health(
    worker: Option<&WorkerHealth>,
    runtime_running: bool,
    credentials_ok: bool,
    credential_reason: Option<&str>,
    slack_enabled: bool,
) -> (String, String) {
    if !slack_enabled {
        return (
            "disabled".to_string(),
            "slack channel disabled in settings".to_string(),
        );
    }

    if !credentials_ok {
        return (
            "auth_missing".to_string(),
            credential_reason
                .unwrap_or("missing or invalid slack credentials")
                .to_string(),
        );
    }

    if !runtime_running {
        return (
            "not_running".to_string(),
            "supervisor is not running".to_string(),
        );
    }

    match worker {
        Some(worker) if worker.state == WorkerState::Running => (
            "running".to_string(),
            "worker heartbeat is healthy".to_string(),
        ),
        Some(worker) if worker.state == WorkerState::Error => {
            let reason = worker
                .last_error
                .clone()
                .unwrap_or_else(|| "slack worker reported an error".to_string());
            if reason.contains("missing required env var")
                || reason.contains("profile-scoped credentials")
            {
                ("auth_missing".to_string(), reason)
            } else {
                ("api_failure".to_string(), reason)
            }
        }
        _ => (
            "api_failure".to_string(),
            "slack worker is enabled but not reporting running health".to_string(),
        ),
    }
}

fn slack_profile_status_lines(settings: &Settings, state: &SupervisorState) -> Vec<String> {
    let slack_enabled = settings
        .channels
        .get("slack")
        .map(|cfg| cfg.enabled)
        .unwrap_or(false);
    let worker = state.workers.get("channel:slack");

    let mut lines = Vec::new();
    for health in slack::profile_credential_health(settings) {
        let (status, reason) = classify_slack_profile_health(
            worker,
            state.running,
            health.ok,
            health.reason.as_deref(),
            slack_enabled,
        );
        lines.push(format!(
            "slack_profile:{}.health={status}",
            health.profile_id
        ));
        lines.push(format!(
            "slack_profile:{}.reason={reason}",
            health.profile_id
        ));
    }
    lines
}

fn cmd_status() -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let mut state = load_supervisor_state(&paths).map_err(|e| e.to_string())?;
    let mut ownership = "not_running".to_string();
    match supervisor_ownership_state(&paths).map_err(|e| e.to_string())? {
        OwnershipState::Running { pid } => {
            ownership = "running".to_string();
            if !state.running || state.pid != Some(pid) {
                state.running = true;
                state.pid = Some(pid);
                if state.started_at.is_none() {
                    state.started_at = Some(now_secs());
                }
                state.stopped_at = None;
                save_supervisor_state(&paths, &state).map_err(|e| e.to_string())?;
            }
        }
        OwnershipState::Stale => {
            ownership = "stale".to_string();
            cleanup_stale_supervisor(&paths).map_err(|e| e.to_string())?;
            state = load_supervisor_state(&paths).map_err(|e| e.to_string())?;
        }
        OwnershipState::NotRunning => {
            if state.running || state.pid.is_some() {
                cleanup_stale_supervisor(&paths).map_err(|e| e.to_string())?;
                state = load_supervisor_state(&paths).map_err(|e| e.to_string())?;
            }
        }
    }
    let mut lines = Vec::new();
    lines.push(format!("ownership={ownership}"));
    lines.push(format!("running={}", state.running));
    lines.push(format!(
        "pid={}",
        state
            .pid
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    lines.push(format!(
        "started_at={}",
        state
            .started_at
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    lines.push(format!(
        "stopped_at={}",
        state
            .stopped_at
            .map(|v| v.to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    lines.push(format!(
        "last_error={}",
        state
            .last_error
            .clone()
            .unwrap_or_else(|| "none".to_string())
    ));
    for (id, worker) in &state.workers {
        lines.push(format!("worker:{id}.state={:?}", worker.state).to_lowercase());
        lines.push(format!(
            "worker:{id}.last_heartbeat={}",
            worker
                .last_heartbeat
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".to_string())
        ));
        lines.push(format!(
            "worker:{id}.last_error={}",
            worker
                .last_error
                .clone()
                .unwrap_or_else(|| "none".to_string())
        ));
    }
    if let Ok(settings) = load_settings() {
        lines.extend(slack_profile_status_lines(&settings, &state));
    }
    Ok(lines.join("\n"))
}

fn cmd_logs() -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let logs_dir = paths.root.join("logs");
    fs::create_dir_all(&logs_dir)
        .map_err(|e| format!("failed to create {}: {e}", logs_dir.display()))?;

    let mut entries = Vec::new();
    for entry in fs::read_dir(&logs_dir)
        .map_err(|e| format!("failed to read {}: {e}", logs_dir.display()))?
    {
        let entry = entry.map_err(|e| format!("failed to read log entry: {e}"))?;
        let path = entry.path();
        if path.is_file() {
            entries.push(path);
        }
    }
    entries.sort();

    if entries.is_empty() {
        return Ok("no logs".to_string());
    }

    let mut out = Vec::new();
    for path in entries {
        let raw = fs::read_to_string(&path).unwrap_or_default();
        let mut recent = raw.lines().rev().take(3).collect::<Vec<_>>();
        recent.reverse();
        for line in recent {
            out.push(format!("{}: {}", path.display(), line));
        }
    }
    Ok(out.join("\n"))
}

fn cmd_supervisor(args: &[String]) -> Result<String, String> {
    let state_root = parse_supervisor_state_root(args)?;
    let settings = load_settings()?;
    run_supervisor(&state_root, settings).map_err(|e| e.to_string())?;
    Ok("supervisor exited".to_string())
}

fn parse_supervisor_state_root(args: &[String]) -> Result<PathBuf, String> {
    if args.len() == 2 && args[0] == "--state-root" {
        return Ok(PathBuf::from(&args[1]));
    }
    Err("usage: __supervisor --state-root <path>".to_string())
}

fn cmd_send(args: &[String]) -> Result<String, String> {
    if args.len() < 2 {
        return Err("usage: send <channel_profile_id> <message>".to_string());
    }
    let settings = load_settings()?;
    let profile_id = args[0].clone();
    let profile = settings
        .channel_profiles
        .get(&profile_id)
        .ok_or_else(|| format!("unknown channel profile `{profile_id}`"))?;
    let message = args[1..].join(" ");

    let paths = ensure_runtime_root()?;
    let ts = now_secs();
    let msg_id = format!("msg-{}", now_nanos());
    let incoming = IncomingMessage {
        channel: profile.channel.to_string(),
        channel_profile_id: Some(profile_id.clone()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message,
        timestamp: ts,
        message_id: msg_id.clone(),
        conversation_id: None,
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let queue_path = paths
        .root
        .join("queue/incoming")
        .join(format!("{}.json", incoming.message_id));
    let body = serde_json::to_vec_pretty(&incoming)
        .map_err(|e| format!("failed to encode queue message: {e}"))?;
    fs::write(&queue_path, body)
        .map_err(|e| format!("failed to write {}: {e}", queue_path.display()))?;
    Ok(format!("queued\nmessage_id={msg_id}"))
}

fn cmd_channels(args: &[String]) -> Result<String, String> {
    if args.len() == 1 && args[0] == "reset" {
        let paths = ensure_runtime_root()?;
        let channels_dir = paths.root.join("channels");
        if channels_dir.exists() {
            fs::remove_dir_all(&channels_dir)
                .map_err(|e| format!("failed to reset {}: {e}", channels_dir.display()))?;
        }
        fs::create_dir_all(&channels_dir)
            .map_err(|e| format!("failed to create {}: {e}", channels_dir.display()))?;
        return Ok("channels reset complete".to_string());
    }
    if args.len() == 2 && args[0] == "slack" && args[1] == "sync" {
        let paths = ensure_runtime_root()?;
        let settings = load_settings()?;
        let report = slack::sync_once(&paths.root, &settings).map_err(|e| e.to_string())?;
        return Ok(format!(
            "slack sync complete\nprofiles_processed={}\ninbound_enqueued={}\noutbound_messages_sent={}",
            report.profiles_processed, report.inbound_enqueued, report.outbound_messages_sent
        ));
    }
    Err("usage: channels reset | channels slack sync".to_string())
}

fn cmd_orchestrator(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("usage: orchestrator <list|add|show|remove|set-private-workspace|grant-shared-access|revoke-shared-access|set-selector-agent|set-default-workflow|set-selection-max-retries> ...".to_string());
    }

    match args[0].as_str() {
        "list" => {
            let settings = load_settings()?;
            Ok(settings
                .orchestrators
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "add" => {
            if args.len() != 2 {
                return Err("usage: orchestrator add <orchestrator_id>".to_string());
            }
            let mut settings = load_settings()?;
            let id = args[1].clone();
            if settings.orchestrators.contains_key(&id) {
                return Err(format!("orchestrator `{id}` already exists"));
            }
            settings.orchestrators.insert(
                id.clone(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            );
            save_settings(&settings)?;

            let private_workspace = settings
                .resolve_private_workspace(&id)
                .map_err(map_config_err)?;
            fs::create_dir_all(&private_workspace)
                .map_err(|e| format!("failed to create {}: {e}", private_workspace.display()))?;
            let path = save_orchestrator_config(&settings, &id, &default_orchestrator_config(&id))?;
            Ok(format!(
                "orchestrator added\nid={}\nprivate_workspace={}\nconfig={}",
                id,
                private_workspace.display(),
                path.display()
            ))
        }
        "show" => {
            if args.len() != 2 {
                return Err("usage: orchestrator show <orchestrator_id>".to_string());
            }
            let settings = load_settings()?;
            let id = &args[1];
            let entry = settings
                .orchestrators
                .get(id)
                .ok_or_else(|| format!("unknown orchestrator `{id}`"))?;
            let private_workspace = settings
                .resolve_private_workspace(id)
                .map_err(map_config_err)?;
            Ok(format!(
                "id={}\nprivate_workspace={}\nshared_access={}",
                id,
                private_workspace.display(),
                entry.shared_access.join(",")
            ))
        }
        "remove" => {
            if args.len() != 2 {
                return Err("usage: orchestrator remove <orchestrator_id>".to_string());
            }
            let mut settings = load_settings()?;
            let id = args[1].clone();
            if settings.orchestrators.remove(&id).is_none() {
                return Err(format!("unknown orchestrator `{id}`"));
            }
            save_settings(&settings)?;
            remove_orchestrator_config(&settings, &id)?;
            Ok(format!("orchestrator removed\nid={id}"))
        }
        "set-private-workspace" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator set-private-workspace <orchestrator_id> <abs_path>"
                        .to_string(),
                );
            }
            let mut settings = load_settings()?;
            let id = &args[1];
            let orchestrator = load_orchestrator_or_err(&settings, id)?;
            let path = PathBuf::from(&args[2]);
            if !path.is_absolute() {
                return Err("private workspace path must be absolute".to_string());
            }
            let entry = settings
                .orchestrators
                .get_mut(id)
                .ok_or_else(|| format!("unknown orchestrator `{id}`"))?;
            entry.private_workspace = Some(path.clone());
            save_settings(&settings)?;
            save_orchestrator_config(&settings, id, &orchestrator)?;
            Ok(format!(
                "orchestrator updated\nid={}\nprivate_workspace={}",
                id,
                path.display()
            ))
        }
        "grant-shared-access" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator grant-shared-access <orchestrator_id> <shared_key>"
                        .to_string(),
                );
            }
            let mut settings = load_settings()?;
            let id = &args[1];
            let shared_key = args[2].clone();
            if !settings.shared_workspaces.contains_key(&shared_key) {
                return Err(format!("invalid shared key `{shared_key}`"));
            }
            let entry = settings
                .orchestrators
                .get_mut(id)
                .ok_or_else(|| format!("unknown orchestrator `{id}`"))?;
            if !entry.shared_access.contains(&shared_key) {
                entry.shared_access.push(shared_key.clone());
            }
            save_settings(&settings)?;
            Ok(format!(
                "shared access granted\nid={}\nshared_key={}",
                id, shared_key
            ))
        }
        "revoke-shared-access" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator revoke-shared-access <orchestrator_id> <shared_key>"
                        .to_string(),
                );
            }
            let mut settings = load_settings()?;
            let id = &args[1];
            let shared_key = args[2].clone();
            let entry = settings
                .orchestrators
                .get_mut(id)
                .ok_or_else(|| format!("unknown orchestrator `{id}`"))?;
            entry.shared_access.retain(|v| v != &shared_key);
            save_settings(&settings)?;
            Ok(format!(
                "shared access revoked\nid={}\nshared_key={}",
                id, shared_key
            ))
        }
        "set-selector-agent" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator set-selector-agent <orchestrator_id> <agent_id>"
                        .to_string(),
                );
            }
            let settings = load_settings()?;
            let id = &args[1];
            let agent_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, id)?;
            if !orchestrator.agents.contains_key(&agent_id) {
                return Err(format!("unknown agent `{agent_id}`"));
            }
            orchestrator.selector_agent = agent_id.clone();
            save_orchestrator_config(&settings, id, &orchestrator)?;
            Ok(format!(
                "selector updated\nid={}\nselector_agent={}",
                id, agent_id
            ))
        }
        "set-default-workflow" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator set-default-workflow <orchestrator_id> <workflow_id>"
                        .to_string(),
                );
            }
            let settings = load_settings()?;
            let id = &args[1];
            let workflow_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, id)?;
            if !orchestrator.workflows.iter().any(|w| w.id == workflow_id) {
                return Err(format!("invalid workflow id `{workflow_id}`"));
            }
            orchestrator.default_workflow = workflow_id.clone();
            save_orchestrator_config(&settings, id, &orchestrator)?;
            Ok(format!(
                "default workflow updated\nid={}\ndefault_workflow={}",
                id, workflow_id
            ))
        }
        "set-selection-max-retries" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator set-selection-max-retries <orchestrator_id> <count>"
                        .to_string(),
                );
            }
            let settings = load_settings()?;
            let id = &args[1];
            let count: u32 = args[2]
                .parse()
                .map_err(|_| "count must be a positive integer".to_string())?;
            if count == 0 {
                return Err("count must be >= 1".to_string());
            }
            let mut orchestrator = load_orchestrator_or_err(&settings, id)?;
            orchestrator.selection_max_retries = count;
            save_orchestrator_config(&settings, id, &orchestrator)?;
            Ok(format!(
                "selection retries updated\nid={}\ncount={}",
                id, count
            ))
        }
        other => Err(format!("unknown orchestrator subcommand `{other}`")),
    }
}

fn cmd_orchestrator_agent(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("usage: orchestrator-agent <list|add|show|remove|reset> ...".to_string());
    }

    match args[0].as_str() {
        "list" => {
            if args.len() != 2 {
                return Err("usage: orchestrator-agent list <orchestrator_id>".to_string());
            }
            let settings = load_settings()?;
            let orchestrator = load_orchestrator_or_err(&settings, &args[1])?;
            Ok(orchestrator
                .agents
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "add" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator-agent add <orchestrator_id> <agent_id>".to_string()
                );
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let agent_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            if orchestrator.agents.contains_key(&agent_id) {
                return Err(format!("agent `{agent_id}` already exists"));
            }
            let private_workspace = settings
                .resolve_private_workspace(orchestrator_id)
                .map_err(map_config_err)?
                .join("agents")
                .join(&agent_id);
            fs::create_dir_all(&private_workspace)
                .map_err(|e| format!("failed to create {}: {e}", private_workspace.display()))?;
            orchestrator.agents.insert(
                agent_id.clone(),
                AgentConfig {
                    provider: ConfigProviderKind::Anthropic,
                    model: "sonnet".to_string(),
                    private_workspace: Some(private_workspace),
                    can_orchestrate_workflows: false,
                    shared_access: Vec::new(),
                },
            );
            save_orchestrator_config(&settings, orchestrator_id, &orchestrator)?;
            Ok(format!(
                "agent added\norchestrator={}\nagent={}",
                orchestrator_id, agent_id
            ))
        }
        "show" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator-agent show <orchestrator_id> <agent_id>".to_string(),
                );
            }
            let settings = load_settings()?;
            let orchestrator = load_orchestrator_or_err(&settings, &args[1])?;
            let agent = orchestrator
                .agents
                .get(&args[2])
                .ok_or_else(|| format!("unknown agent `{}`", args[2]))?;
            Ok(format!(
                "id={}\nprovider={}\nmodel={}\ncan_orchestrate_workflows={}",
                args[2], agent.provider, agent.model, agent.can_orchestrate_workflows
            ))
        }
        "remove" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator-agent remove <orchestrator_id> <agent_id>".to_string(),
                );
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let agent_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            if orchestrator.agents.remove(&agent_id).is_none() {
                return Err(format!("unknown agent `{agent_id}`"));
            }
            save_orchestrator_config(&settings, orchestrator_id, &orchestrator)?;
            Ok(format!(
                "agent removed\norchestrator={}\nagent={}",
                orchestrator_id, agent_id
            ))
        }
        "reset" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator-agent reset <orchestrator_id> <agent_id>".to_string(),
                );
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let agent_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            let agent = orchestrator
                .agents
                .get_mut(&agent_id)
                .ok_or_else(|| format!("unknown agent `{agent_id}`"))?;
            agent.provider = ConfigProviderKind::Anthropic;
            agent.model = "sonnet".to_string();
            agent.can_orchestrate_workflows = false;
            save_orchestrator_config(&settings, orchestrator_id, &orchestrator)?;
            Ok(format!(
                "agent reset\norchestrator={}\nagent={}",
                orchestrator_id, agent_id
            ))
        }
        other => Err(format!("unknown orchestrator-agent subcommand `{other}`")),
    }
}

fn cmd_workflow(args: &[String]) -> Result<String, String> {
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
            let mut orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            if orchestrator.workflows.iter().any(|w| w.id == workflow_id) {
                return Err(format!("workflow `{workflow_id}` already exists"));
            }
            let selector = orchestrator.selector_agent.clone();
            orchestrator.workflows.push(WorkflowConfig {
                id: workflow_id.clone(),
                version: 1,
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
            let paths = ensure_runtime_root()?;
            let store = WorkflowRunStore::new(&paths.root);
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
            let paths = ensure_runtime_root()?;
            let store = WorkflowRunStore::new(&paths.root);
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
            let paths = ensure_runtime_root()?;
            let store = WorkflowRunStore::new(&paths.root);
            let progress = store.load_progress(&args[1]).map_err(|e| e.to_string())?;
            serde_json::to_string_pretty(&progress)
                .map_err(|e| format!("failed to encode workflow progress: {e}"))
        }
        "cancel" => {
            if args.len() != 2 {
                return Err("usage: workflow cancel <run_id>".to_string());
            }
            let paths = ensure_runtime_root()?;
            let store = WorkflowRunStore::new(&paths.root);
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

fn cmd_channel_profile(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err(
            "usage: channel-profile <list|add|show|remove|set-orchestrator> ...".to_string(),
        );
    }

    match args[0].as_str() {
        "list" => {
            let settings = load_settings()?;
            Ok(settings
                .channel_profiles
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "add" => {
            if args.len() < 4 {
                return Err("usage: channel-profile add <channel_profile_id> <channel> <orchestrator_id> [--slack-app-user-id <id>] [--require-mention-in-channels <bool>]".to_string());
            }
            let mut settings = load_settings()?;
            let id = args[1].clone();
            let channel = ChannelKind::parse(&args[2])?;
            let orchestrator_id = args[3].clone();
            if !settings.orchestrators.contains_key(&orchestrator_id) {
                return Err(format!("unknown orchestrator `{orchestrator_id}`"));
            }
            if settings.channel_profiles.contains_key(&id) {
                return Err(format!("channel profile `{id}` already exists"));
            }

            let mut slack_app_user_id = None;
            let mut require_mention = None;
            let mut i = 4usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--slack-app-user-id" => {
                        if i + 1 >= args.len() {
                            return Err("missing value for --slack-app-user-id".to_string());
                        }
                        slack_app_user_id = Some(args[i + 1].clone());
                        i += 2;
                    }
                    "--require-mention-in-channels" => {
                        if i + 1 >= args.len() {
                            return Err(
                                "missing value for --require-mention-in-channels".to_string()
                            );
                        }
                        require_mention = Some(parse_bool(&args[i + 1])?);
                        i += 2;
                    }
                    other => return Err(format!("unknown option `{other}`")),
                }
            }

            settings.channel_profiles.insert(
                id.clone(),
                ChannelProfile {
                    channel,
                    orchestrator_id,
                    slack_app_user_id,
                    require_mention_in_channels: require_mention,
                },
            );
            save_settings(&settings)?;
            Ok(format!("channel profile added\nid={id}"))
        }
        "show" => {
            if args.len() != 2 {
                return Err("usage: channel-profile show <channel_profile_id>".to_string());
            }
            let settings = load_settings()?;
            let profile = settings
                .channel_profiles
                .get(&args[1])
                .ok_or_else(|| format!("unknown channel profile `{}`", args[1]))?;
            let mention_policy = profile
                .require_mention_in_channels
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".to_string());
            Ok(format!(
                "id={}\nchannel={}\norchestrator_id={}\nslack_app_user_id={}\nrequire_mention_in_channels={}",
                args[1],
                profile.channel,
                profile.orchestrator_id,
                profile
                    .slack_app_user_id
                    .clone()
                    .unwrap_or_else(|| "n/a".to_string()),
                mention_policy
            ))
        }
        "remove" => {
            if args.len() != 2 {
                return Err("usage: channel-profile remove <channel_profile_id>".to_string());
            }
            let mut settings = load_settings()?;
            let id = args[1].clone();
            if settings.channel_profiles.remove(&id).is_none() {
                return Err(format!("unknown channel profile `{id}`"));
            }
            save_settings(&settings)?;
            Ok(format!("channel profile removed\nid={id}"))
        }
        "set-orchestrator" => {
            if args.len() != 3 {
                return Err("usage: channel-profile set-orchestrator <channel_profile_id> <orchestrator_id>".to_string());
            }
            let mut settings = load_settings()?;
            let profile_id = &args[1];
            let orchestrator_id = args[2].clone();
            if !settings.orchestrators.contains_key(&orchestrator_id) {
                return Err(format!("unknown orchestrator `{orchestrator_id}`"));
            }
            let profile = settings
                .channel_profiles
                .get_mut(profile_id)
                .ok_or_else(|| format!("unknown channel profile `{profile_id}`"))?;
            profile.orchestrator_id = orchestrator_id.clone();
            save_settings(&settings)?;
            Ok(format!(
                "channel profile updated\nid={}\norchestrator_id={}",
                profile_id, orchestrator_id
            ))
        }
        other => Err(format!("unknown channel-profile subcommand `{other}`")),
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

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected boolean true|false, got `{raw}`")),
    }
}

fn default_orchestrator_config(id: &str) -> OrchestratorConfig {
    initial_orchestrator_config(id, "anthropic", "sonnet", WorkflowTemplate::Minimal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthSyncConfig, Monitoring};
    use tempfile::tempdir;

    #[test]
    fn shared_executor_routes_cli_style_function_calls() {
        let err = execute_function_invocation(
            function_ids::UPDATE_APPLY,
            &Map::new(),
            FunctionExecutionContext {
                run_store: None,
                settings: None,
            },
        )
        .expect_err("update.apply should remain blocked");

        assert!(err.to_string().contains("update apply is unsupported"));
    }

    #[test]
    fn shared_executor_handles_internal_functions() {
        let temp = tempdir().expect("tempdir");
        let settings = Settings {
            workspaces_path: temp.path().to_path_buf(),
            shared_workspaces: BTreeMap::new(),
            orchestrators: BTreeMap::from_iter([(
                "main".to_string(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            )]),
            channel_profiles: BTreeMap::new(),
            monitoring: Monitoring::default(),
            channels: BTreeMap::new(),
            auth_sync: AuthSyncConfig::default(),
        };

        let value = execute_function_invocation(
            function_ids::ORCHESTRATOR_LIST,
            &Map::new(),
            FunctionExecutionContext {
                run_store: None,
                settings: Some(&settings),
            },
        )
        .expect("internal function result");

        let orchestrators = value
            .get("orchestrators")
            .and_then(|v| v.as_array())
            .expect("orchestrators array");
        assert_eq!(orchestrators, &vec![Value::String("main".to_string())]);
    }
}
