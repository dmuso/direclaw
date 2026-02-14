use crate::config::{
    default_global_config_path, default_orchestrators_config_path, load_orchestrator_config,
    AgentConfig, AuthSyncConfig, AuthSyncSource, ChannelProfile, ConfigError, OrchestratorConfig,
    Settings, SettingsOrchestrator, ValidationOptions, WorkflowConfig, WorkflowStepConfig,
};
use crate::orchestrator::{RunState, WorkflowRunStore};
use crate::queue::IncomingMessage;
use crate::runtime::{
    append_runtime_log, bootstrap_state_root, cleanup_stale_supervisor, default_state_root_path,
    load_supervisor_state, reserve_start_lock, run_supervisor, save_supervisor_state,
    spawn_supervisor_process, stop_active_supervisor, supervisor_ownership_state, OwnershipState,
    StatePaths, SupervisorState, WorkerHealth, WorkerState,
};
use crate::slack;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

mod setup_tui;
use self::setup_tui::SetupWorkflowBundle;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RuntimePreferences {
    provider: Option<String>,
    model: Option<String>,
}

pub fn run(args: Vec<String>) -> Result<String, String> {
    if args.is_empty() {
        return Ok(help_text());
    }

    match args[0].as_str() {
        "setup" => setup_tui::cmd_setup(),
        "start" => cmd_start(),
        "stop" => cmd_stop(),
        "restart" => cmd_restart(),
        "status" => cmd_status(),
        "logs" => cmd_logs(),
        "send" => cmd_send(&args[1..]),
        "update" => cmd_update(&args[1..]),
        "doctor" => cmd_doctor(),
        "attach" => cmd_attach(),
        "channels" => cmd_channels(&args[1..]),
        "provider" => cmd_provider(&args[1..]),
        "model" => cmd_model(&args[1..]),
        "agent" => cmd_orchestrator_agent(&args[1..]),
        "orchestrator" => cmd_orchestrator(&args[1..]),
        "orchestrator-agent" => cmd_orchestrator_agent(&args[1..]),
        "workflow" => cmd_workflow(&args[1..]),
        "channel-profile" => cmd_channel_profile(&args[1..]),
        "auth" => cmd_auth(&args[1..]),
        "__supervisor" => cmd_supervisor(&args[1..]),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn help_text() -> String {
    [
        "Commands:",
        "  setup                                Initialize state/config/runtime directories",
        "  start                                Start the DireClaw supervisor and workers",
        "  stop                                 Stop the active supervisor",
        "  restart                              Restart the supervisor and workers",
        "  status                               Show runtime ownership/health status",
        "  logs                                 Print runtime and worker logs",
        "  attach                               Attach to the active runtime session",
        "  doctor                               Run local environment and config checks",
        "  update check|apply                   Check for updates (apply is intentionally blocked)",
        "  send <profile> <message>             Queue a message for a channel profile",
        "  channels reset                       Reset channel sync state",
        "  channels slack sync                  Pull Slack messages into the queue",
        "  auth sync                            Sync provider auth from configured sources",
        "  orchestrator ...                     Manage orchestrators and routing defaults",
        "  orchestrator-agent ...               Manage agents under an orchestrator",
        "  agent ...                            Alias for `orchestrator-agent ...`",
        "  workflow ...                         Manage workflows and workflow runs",
        "  channel-profile ...                  Manage channel-to-orchestrator bindings",
        "  provider ...                         Set/show default provider preference",
        "  model ...                            Set/show default model preference",
    ]
    .join("\n")
}

#[derive(Debug, Clone, Default)]
struct AuthSyncResult {
    synced_sources: Vec<String>,
}

fn render_auth_sync_result(result: &AuthSyncResult, command_context: bool) -> String {
    if result.synced_sources.is_empty() {
        if command_context {
            return "auth sync skipped\nauth_sync_enabled=false".to_string();
        }
        return "auth_sync=skipped".to_string();
    }

    if command_context {
        return format!(
            "auth sync complete\nsynced={}\nsources={}",
            result.synced_sources.len(),
            result.synced_sources.join(",")
        );
    }
    format!("auth_sync=synced({})", result.synced_sources.join(","))
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

fn map_config_err(err: ConfigError) -> String {
    err.to_string()
}

fn state_root() -> Result<PathBuf, String> {
    default_state_root_path().map_err(|e| e.to_string())
}

fn ensure_runtime_root() -> Result<StatePaths, String> {
    let root = state_root()?;
    let paths = StatePaths::new(root);
    bootstrap_state_root(&paths).map_err(|e| e.to_string())?;
    Ok(paths)
}

fn preferences_path(paths: &StatePaths) -> PathBuf {
    paths.root.join("runtime/preferences.yaml")
}

fn load_preferences(paths: &StatePaths) -> Result<RuntimePreferences, String> {
    let path = preferences_path(paths);
    if !path.exists() {
        return Ok(RuntimePreferences::default());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_yaml::from_str(&raw).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

fn save_preferences(paths: &StatePaths, prefs: &RuntimePreferences) -> Result<(), String> {
    let path = preferences_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let body =
        serde_yaml::to_string(prefs).map_err(|e| format!("failed to encode preferences: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

fn load_settings() -> Result<Settings, String> {
    let path = default_global_config_path().map_err(map_config_err)?;
    let settings = Settings::from_path(&path).map_err(map_config_err)?;
    settings
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .map_err(map_config_err)?;
    Ok(settings)
}

fn save_settings(settings: &Settings) -> Result<PathBuf, String> {
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
    let path = default_orchestrators_config_path().map_err(map_config_err)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let mut registry = if path.exists() {
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        serde_yaml::from_str::<BTreeMap<String, OrchestratorConfig>>(&raw)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?
    } else {
        BTreeMap::new()
    };
    registry.insert(orchestrator_id.to_string(), orchestrator.clone());
    let body = serde_yaml::to_string(&registry)
        .map_err(|e| format!("failed to encode orchestrators: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(path)
}

fn save_orchestrator_registry(
    settings: &Settings,
    registry: &BTreeMap<String, OrchestratorConfig>,
) -> Result<PathBuf, String> {
    for (orchestrator_id, orchestrator) in registry {
        orchestrator
            .validate(settings, orchestrator_id)
            .map_err(map_config_err)?;
        let private_workspace = settings
            .resolve_private_workspace(orchestrator_id)
            .map_err(map_config_err)?;
        fs::create_dir_all(&private_workspace)
            .map_err(|e| format!("failed to create {}: {e}", private_workspace.display()))?;
    }
    let path = default_orchestrators_config_path().map_err(map_config_err)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let body = serde_yaml::to_string(registry)
        .map_err(|e| format!("failed to encode orchestrators: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(path)
}

fn remove_orchestrator_config(orchestrator_id: &str) -> Result<(), String> {
    let path = default_orchestrators_config_path().map_err(map_config_err)?;
    if !path.exists() {
        return Ok(());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let mut registry = serde_yaml::from_str::<BTreeMap<String, OrchestratorConfig>>(&raw)
        .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
    registry.remove(orchestrator_id);
    let body = serde_yaml::to_string(&registry)
        .map_err(|e| format!("failed to encode orchestrators: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))
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
        channel: profile.channel.clone(),
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

fn cmd_auth(args: &[String]) -> Result<String, String> {
    if args.len() == 1 && args[0] == "sync" {
        let settings = load_settings()?;
        let result = sync_auth_sources(&settings)?;
        return Ok(render_auth_sync_result(&result, true));
    }
    Err("usage: auth sync".to_string())
}

fn resolve_auth_destination(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is unavailable".to_string())?;
        return Ok(home.join(rest));
    }
    Err(format!(
        "auth destination `{}` must be absolute or start with `~/`",
        path.display()
    ))
}

fn op_service_token() -> Result<String, String> {
    let raw = std::env::var("OP_SERVICE_ACCOUNT_TOKEN")
        .map_err(|_| "OP_SERVICE_ACCOUNT_TOKEN is required for auth sync".to_string())?;
    if raw.trim().is_empty() {
        return Err("OP_SERVICE_ACCOUNT_TOKEN is required for auth sync".to_string());
    }
    Ok(raw)
}

fn sync_onepassword_source(
    source_id: &str,
    source: &AuthSyncSource,
    token: &str,
) -> Result<(), String> {
    let destination = resolve_auth_destination(&source.destination)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }

    let output = Command::new("op")
        .arg("read")
        .arg(&source.reference)
        .env("OP_SERVICE_ACCOUNT_TOKEN", token)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "auth sync failed: `op` binary is not available in PATH".to_string()
            } else {
                format!("auth sync source `{source_id}` failed to execute op: {e}")
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.trim();
        if reason.is_empty() {
            return Err(format!(
                "auth sync source `{source_id}` failed to read `{}`",
                source.reference
            ));
        }
        return Err(format!(
            "auth sync source `{source_id}` failed to read `{}`: {}",
            source.reference, reason
        ));
    }

    if output.stdout.is_empty() {
        return Err(format!(
            "auth sync source `{source_id}` returned empty content"
        ));
    }

    let file_name = destination
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| {
            format!(
                "auth sync source `{source_id}` destination `{}` must include a file name",
                destination.display()
            )
        })?;
    let temp_path = destination.with_file_name(format!(".{file_name}.tmp-{}", now_nanos()));
    fs::write(&temp_path, &output.stdout)
        .map_err(|e| format!("failed to write {}: {e}", temp_path.display()))?;

    #[cfg(unix)]
    if source.owner_only {
        let mut perms = fs::metadata(&temp_path)
            .map_err(|e| format!("failed to read {}: {e}", temp_path.display()))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&temp_path, perms)
            .map_err(|e| format!("failed to chmod {}: {e}", temp_path.display()))?;
    }

    fs::rename(&temp_path, &destination).map_err(|e| {
        let _ = fs::remove_file(&temp_path);
        format!(
            "failed to replace {} with {}: {e}",
            destination.display(),
            temp_path.display()
        )
    })?;
    Ok(())
}

fn sync_auth_sources(settings: &Settings) -> Result<AuthSyncResult, String> {
    if !settings.auth_sync.enabled {
        return Ok(AuthSyncResult::default());
    }

    let mut result = AuthSyncResult::default();
    let token = op_service_token()?;

    for (source_id, source) in &settings.auth_sync.sources {
        match source.backend.trim() {
            "onepassword" => sync_onepassword_source(source_id, source, &token)?,
            other => {
                return Err(format!(
                    "auth sync source `{source_id}` has unsupported backend `{other}`"
                ))
            }
        }
        result.synced_sources.push(source_id.clone());
    }
    Ok(result)
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

fn cmd_update(args: &[String]) -> Result<String, String> {
    if args.is_empty() || args[0] == "check" {
        return cmd_update_check();
    }
    if args[0] == "apply" {
        return Err(
            "update apply is unsupported in this build to avoid unsafe in-place upgrades. remediation: visit GitHub Releases, download the target archive, verify SHA256, and replace the binary manually".to_string(),
        );
    }
    Err("usage: update [check|apply]".to_string())
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GithubLatestRelease {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
    prerelease: bool,
    draft: bool,
    assets: Vec<GithubReleaseAsset>,
}

fn normalize_version_for_compare(raw: &str) -> String {
    raw.trim().trim_start_matches('v').to_ascii_lowercase()
}

fn parse_version_numbers(raw: &str) -> Option<Vec<u64>> {
    let trimmed = normalize_version_for_compare(raw);
    let core = trimmed
        .split_once('-')
        .map(|(left, _)| left)
        .unwrap_or(&trimmed);
    if core.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    for part in core.split('.') {
        if part.is_empty() {
            return None;
        }
        out.push(part.parse::<u64>().ok()?);
    }
    Some(out)
}

fn is_update_available(current: &str, latest: &str) -> bool {
    if let (Some(mut current_parts), Some(mut latest_parts)) = (
        parse_version_numbers(current),
        parse_version_numbers(latest),
    ) {
        let max_len = current_parts.len().max(latest_parts.len());
        current_parts.resize(max_len, 0);
        latest_parts.resize(max_len, 0);
        return latest_parts > current_parts;
    }
    normalize_version_for_compare(current) != normalize_version_for_compare(latest)
}

fn update_repo() -> String {
    std::env::var("DIRECLAW_UPDATE_REPO").unwrap_or_else(|_| "dharper/rustyclaw".to_string())
}

fn update_api_base() -> String {
    std::env::var("DIRECLAW_UPDATE_API_URL")
        .unwrap_or_else(|_| "https://api.github.com".to_string())
        .trim_end_matches('/')
        .to_string()
}

fn load_latest_release(repo: &str) -> Result<GithubLatestRelease, String> {
    let (owner, name) = repo.split_once('/').ok_or_else(|| {
        format!("update check failed: repository `{repo}` must use `owner/name` format")
    })?;
    let url = format!(
        "{}/repos/{}/{}/releases/latest",
        update_api_base(),
        urlencoding::encode(owner),
        urlencoding::encode(name)
    );
    let response = ureq::get(&url)
        .set("accept", "application/vnd.github+json")
        .set(
            "user-agent",
            concat!("direclaw/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| format!("update check failed to query {url}: {e}"))?;

    let status = response.status();
    if status != 200 {
        return Err(format!(
            "update check failed to query {url}: unexpected status {status}"
        ));
    }

    response
        .into_json::<GithubLatestRelease>()
        .map_err(|e| format!("update check failed to parse release metadata: {e}"))
}

fn cmd_update_check() -> Result<String, String> {
    let repo = update_repo();
    let release = load_latest_release(&repo).map_err(|err| {
        format!(
            "{err}. remediation: verify network access and set DIRECLAW_UPDATE_REPO/DIRECLAW_UPDATE_API_URL if needed"
        )
    })?;
    if release.draft {
        return Err(
            "update check failed: latest release is a draft and cannot be used for updates"
                .to_string(),
        );
    }

    let current_version = env!("CARGO_PKG_VERSION");
    let latest_version = normalize_version_for_compare(&release.tag_name);
    let update_available = is_update_available(current_version, &latest_version);

    let mut lines = vec![
        "update_check=ok".to_string(),
        format!("repository={repo}"),
        format!("current_version={current_version}"),
        format!("latest_version={latest_version}"),
        format!("release_tag={}", release.tag_name),
        format!("release_url={}", release.html_url),
        format!("prerelease={}", release.prerelease),
        format!("update_available={update_available}"),
    ];
    if let Some(published_at) = release.published_at {
        lines.push(format!("published_at={published_at}"));
    }
    if !release.assets.is_empty() {
        let mut names: Vec<String> = release.assets.into_iter().map(|asset| asset.name).collect();
        names.sort();
        lines.push(format!("assets={}", names.join(",")));
    }
    if update_available {
        lines.push(
            "remediation=download release archive, verify SHA256 from checksums.txt, replace binary manually"
                .to_string(),
        );
    }
    Ok(lines.join("\n"))
}

#[derive(Debug, Clone)]
struct DoctorFinding {
    id: String,
    ok: bool,
    detail: String,
    remediation: String,
}

fn doctor_finding(
    id: impl Into<String>,
    ok: bool,
    detail: impl Into<String>,
    remediation: impl Into<String>,
) -> DoctorFinding {
    DoctorFinding {
        id: id.into(),
        ok,
        detail: detail.into(),
        remediation: remediation.into(),
    }
}

fn is_binary_available(binary: &str) -> bool {
    if binary.trim().is_empty() {
        return false;
    }
    let explicit = Path::new(binary);
    if explicit.components().count() > 1 || explicit.is_absolute() {
        return is_executable_file(explicit);
    }

    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(binary);
        if is_executable_file(&candidate) {
            return true;
        }
        #[cfg(windows)]
        {
            if is_executable_file(&dir.join(format!("{binary}.exe"))) {
                return true;
            }
        }
        false
    })
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn can_write_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    let probe = path.join(format!(".direclaw-doctor-{}", now_nanos()));
    fs::write(&probe, b"ok").map_err(|e| format!("failed to write {}: {e}", probe.display()))?;
    fs::remove_file(&probe).map_err(|e| format!("failed to remove {}: {e}", probe.display()))
}

fn cmd_doctor() -> Result<String, String> {
    let mut findings = Vec::new();
    let config_path = default_global_config_path().map_err(map_config_err)?;
    findings.push(doctor_finding(
        "config.path",
        config_path.exists(),
        format!("config={}", config_path.display()),
        "run `direclaw setup` to create default config",
    ));

    let settings = match load_settings() {
        Ok(settings) => {
            findings.push(doctor_finding(
                "config.parse",
                true,
                "settings parsed and validated",
                "none",
            ));
            Some(settings)
        }
        Err(err) => {
            findings.push(doctor_finding(
                "config.parse",
                false,
                format!("settings load failed: {err}"),
                "fix ~/.direclaw/config.yaml and retry `direclaw doctor`",
            ));
            None
        }
    };

    let anthropic_bin = std::env::var("DIRECLAW_PROVIDER_BIN_ANTHROPIC")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "claude".to_string());
    let openai_bin = std::env::var("DIRECLAW_PROVIDER_BIN_OPENAI")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "codex".to_string());
    findings.push(doctor_finding(
        "binary.anthropic",
        is_binary_available(&anthropic_bin),
        format!("binary={anthropic_bin}"),
        "install the Anthropic CLI or set DIRECLAW_PROVIDER_BIN_ANTHROPIC",
    ));
    findings.push(doctor_finding(
        "binary.openai",
        is_binary_available(&openai_bin),
        format!("binary={openai_bin}"),
        "install the OpenAI Codex CLI or set DIRECLAW_PROVIDER_BIN_OPENAI",
    ));

    if let Some(settings) = settings.as_ref() {
        findings.push(match can_write_directory(&settings.workspaces_path) {
            Ok(_) => doctor_finding(
                "workspace.root",
                true,
                format!("writable={}", settings.workspaces_path.display()),
                "none",
            ),
            Err(err) => doctor_finding(
                "workspace.root",
                false,
                err,
                "grant write permission to workspaces_path in ~/.direclaw/config.yaml",
            ),
        });
        let orchestrators_config_path =
            default_orchestrators_config_path().map_err(map_config_err)?;
        let orchestrator_registry_required = !settings.orchestrators.is_empty();
        findings.push(doctor_finding(
            "config.orchestrators.path",
            !orchestrator_registry_required || orchestrators_config_path.exists(),
            format!(
                "path={} required={}",
                orchestrators_config_path.display(),
                orchestrator_registry_required
            ),
            "run `direclaw setup` or create ~/.direclaw/config-orchestrators.yaml",
        ));

        for orchestrator_id in settings.orchestrators.keys() {
            match settings.resolve_private_workspace(orchestrator_id) {
                Ok(private_workspace) => {
                    findings.push(match can_write_directory(&private_workspace) {
                        Ok(_) => doctor_finding(
                            format!("workspace.orchestrator.{orchestrator_id}"),
                            true,
                            format!("writable={}", private_workspace.display()),
                            "none",
                        ),
                        Err(err) => doctor_finding(
                            format!("workspace.orchestrator.{orchestrator_id}"),
                            false,
                            err,
                            format!(
                                "grant write permission or update `orchestrators.{orchestrator_id}.private_workspace`"
                            ),
                        ),
                    });
                    findings.push(doctor_finding(
                        format!("config.orchestrator.{orchestrator_id}"),
                        load_orchestrator_or_err(settings, orchestrator_id).is_ok(),
                        format!("source={}", orchestrators_config_path.display()),
                        format!(
                            "run `direclaw orchestrator add {orchestrator_id}` or add `{orchestrator_id}` in ~/.direclaw/config-orchestrators.yaml"
                        ),
                    ));
                }
                Err(err) => findings.push(doctor_finding(
                    format!("workspace.orchestrator.{orchestrator_id}"),
                    false,
                    err.to_string(),
                    "fix orchestrator private workspace config",
                )),
            }
        }

        if settings.auth_sync.enabled {
            let token_ok = std::env::var("OP_SERVICE_ACCOUNT_TOKEN")
                .ok()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
            findings.push(doctor_finding(
                "env.OP_SERVICE_ACCOUNT_TOKEN",
                token_ok,
                "required for auth sync",
                "set OP_SERVICE_ACCOUNT_TOKEN before running `direclaw auth sync`",
            ));
            findings.push(doctor_finding(
                "binary.op",
                is_binary_available("op"),
                "required for auth sync backend onepassword",
                "install 1Password CLI (`op`) and ensure PATH includes it",
            ));
        }

        if settings
            .channels
            .get("slack")
            .map(|cfg| cfg.enabled)
            .unwrap_or(false)
        {
            findings.push(match slack::validate_startup_credentials(settings) {
                Ok(_) => doctor_finding("env.slack", true, "slack credentials validated", "none"),
                Err(err) => doctor_finding(
                    "env.slack",
                    false,
                    err.to_string(),
                    "set required SLACK_* env vars for each configured slack profile",
                ),
            });
        }
    }

    let failed = findings.iter().filter(|f| !f.ok).count();
    let summary = if failed == 0 { "healthy" } else { "unhealthy" };
    let mut lines = vec![
        format!("summary={summary}"),
        format!("checks_total={}", findings.len()),
        format!("checks_failed={failed}"),
    ];
    for finding in findings {
        lines.push(format!(
            "check:{}={}",
            finding.id,
            if finding.ok { "ok" } else { "fail" }
        ));
        lines.push(format!("check:{}.detail={}", finding.id, finding.detail));
        if !finding.ok {
            lines.push(format!(
                "check:{}.remediation={}",
                finding.id, finding.remediation
            ));
        }
    }
    Ok(lines.join("\n"))
}

fn cmd_attach() -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let state = load_supervisor_state(&paths).map_err(|e| e.to_string())?;
    if state.running {
        return Ok("attached=true\nsummary=connected to supervisor runtime".to_string());
    }

    let runs_dir = paths.root.join("workflows/runs");
    let mut count = 0usize;
    if runs_dir.exists() {
        for entry in fs::read_dir(&runs_dir)
            .map_err(|e| format!("failed to read {}: {e}", runs_dir.display()))?
        {
            let path = entry
                .map_err(|e| format!("failed to read workflow entry: {e}"))?
                .path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                count += 1;
            }
        }
    }

    Ok(format!("attached=false\nsummary=workflow_runs={count}"))
}

fn cmd_provider(args: &[String]) -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let mut prefs = load_preferences(&paths)?;

    if args.is_empty() {
        return Ok(format!(
            "provider={}\nmodel={}",
            prefs.provider.unwrap_or_else(|| "none".to_string()),
            prefs.model.unwrap_or_else(|| "none".to_string())
        ));
    }

    let provider = args[0].clone();
    if provider != "anthropic" && provider != "openai" {
        return Err("provider must be one of: anthropic, openai".to_string());
    }

    prefs.provider = Some(provider.clone());
    if args.len() >= 3 && args[1] == "--model" {
        prefs.model = Some(args[2].clone());
    }
    save_preferences(&paths, &prefs)?;

    Ok(format!(
        "provider={}\nmodel={}",
        provider,
        prefs.model.unwrap_or_else(|| "none".to_string())
    ))
}

fn cmd_model(args: &[String]) -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let mut prefs = load_preferences(&paths)?;

    if args.is_empty() {
        return Ok(format!(
            "model={}",
            prefs.model.unwrap_or_else(|| "none".to_string())
        ));
    }

    prefs.model = Some(args[0].clone());
    save_preferences(&paths, &prefs)?;
    Ok(format!("model={}", args[0]))
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
            remove_orchestrator_config(&id)?;
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
            let old_private_workspace = settings.resolve_private_workspace(id).ok();
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
            let _ = old_private_workspace;
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
                    provider: "anthropic".to_string(),
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
            agent.provider = "anthropic".to_string();
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
                inputs: serde_yaml::Value::Sequence(Vec::new()),
                limits: None,
                steps: vec![WorkflowStepConfig {
                    id: "step_1".to_string(),
                    step_type: "agent_task".to_string(),
                    agent: selector,
                    prompt: "placeholder".to_string(),
                    next: None,
                    on_approve: None,
                    on_reject: None,
                    outputs: None,
                    output_files: None,
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
            let mut run = store
                .create_run(run_id.clone(), workflow_id.clone(), now_secs())
                .map_err(|e| e.to_string())?;
            store
                .transition_state(
                    &mut run,
                    RunState::Running,
                    now_secs(),
                    format!("workflow started with {} inputs", input_map.len()),
                    false,
                    "continue workflow",
                )
                .map_err(|e| e.to_string())?;
            Ok(format!("workflow started\nrun_id={run_id}"))
        }
        "status" => {
            if args.len() != 2 {
                return Err("usage: workflow status <run_id>".to_string());
            }
            let paths = ensure_runtime_root()?;
            let store = WorkflowRunStore::new(&paths.root);
            let progress = store.load_progress(&args[1]).map_err(|e| e.to_string())?;
            Ok(format!(
                "run_id={}\nstate={}\nsummary={}",
                progress.run_id, progress.state, progress.summary
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
            let channel = args[2].clone();
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
        if key.trim().is_empty() {
            return Err("input key cannot be empty".to_string());
        }
        map.insert(key.to_string(), Value::String(value.to_string()));
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

fn workflow_step(id: &str, step_type: &str, agent: &str, prompt: &str) -> WorkflowStepConfig {
    WorkflowStepConfig {
        id: id.to_string(),
        step_type: step_type.to_string(),
        agent: agent.to_string(),
        prompt: prompt.to_string(),
        next: None,
        on_approve: None,
        on_reject: None,
        outputs: None,
        output_files: None,
        limits: None,
    }
}

fn agent_config(
    provider: &str,
    model: &str,
    private_workspace: &str,
    can_orchestrate_workflows: bool,
) -> AgentConfig {
    AgentConfig {
        provider: provider.to_string(),
        model: model.to_string(),
        private_workspace: Some(Path::new(private_workspace).to_path_buf()),
        can_orchestrate_workflows,
        shared_access: Vec::new(),
    }
}

fn initial_orchestrator_config(
    id: &str,
    provider: &str,
    model: &str,
    bundle: SetupWorkflowBundle,
) -> OrchestratorConfig {
    let selector = "default".to_string();
    let mut agents = BTreeMap::from_iter([(
        selector.clone(),
        agent_config(provider, model, "agents/default", true),
    )]);
    let (default_workflow, workflows) = match bundle {
        SetupWorkflowBundle::Minimal => {
            let workflow_id = "default".to_string();
            let steps = vec![workflow_step(
                "step_1",
                "agent_task",
                &selector,
                "You are the default workflow step.",
            )];
            (
                workflow_id.clone(),
                vec![WorkflowConfig {
                    id: workflow_id,
                    version: 1,
                    inputs: serde_yaml::Value::Sequence(Vec::new()),
                    limits: None,
                    steps,
                }],
            )
        }
        SetupWorkflowBundle::Engineering => {
            agents.insert(
                "planner".to_string(),
                agent_config(provider, model, "agents/planner", false),
            );
            agents.insert(
                "builder".to_string(),
                agent_config(provider, model, "agents/builder", false),
            );
            agents.insert(
                "reviewer".to_string(),
                agent_config(provider, model, "agents/reviewer", false),
            );

            let mut review = workflow_step(
                "review",
                "agent_review",
                "reviewer",
                "Review implementation and return approve or reject with concrete feedback.",
            );
            review.on_approve = Some("done".to_string());
            review.on_reject = Some("implement".to_string());

            let mut implement = workflow_step(
                "implement",
                "agent_task",
                "builder",
                "Implement the approved plan and summarize changed files and test impact.",
            );
            implement.next = Some("review".to_string());

            (
                "feature_delivery".to_string(),
                vec![
                    WorkflowConfig {
                        id: "feature_delivery".to_string(),
                        version: 1,
                        inputs: serde_yaml::Value::Sequence(Vec::new()),
                        limits: None,
                        steps: vec![
                            {
                                let mut plan = workflow_step(
                                    "plan",
                                    "agent_task",
                                    "planner",
                                    "Draft an implementation plan with risks and test strategy.",
                                );
                                plan.next = Some("implement".to_string());
                                plan
                            },
                            implement,
                            review,
                            workflow_step(
                                "done",
                                "agent_task",
                                "planner",
                                "Summarize final outcome and recommended follow-up actions.",
                            ),
                        ],
                    },
                    WorkflowConfig {
                        id: "quick_answer".to_string(),
                        version: 1,
                        inputs: serde_yaml::Value::Sequence(Vec::new()),
                        limits: None,
                        steps: vec![workflow_step(
                            "answer",
                            "agent_task",
                            "planner",
                            "Answer the user request directly and concisely.",
                        )],
                    },
                ],
            )
        }
        SetupWorkflowBundle::Product => {
            agents.insert(
                "researcher".to_string(),
                agent_config(provider, model, "agents/researcher", false),
            );
            agents.insert(
                "writer".to_string(),
                agent_config(provider, model, "agents/writer", false),
            );

            (
                "prd_draft".to_string(),
                vec![
                    WorkflowConfig {
                        id: "prd_draft".to_string(),
                        version: 1,
                        inputs: serde_yaml::Value::Sequence(Vec::new()),
                        limits: None,
                        steps: vec![
                            {
                                let mut research = workflow_step(
                                    "research",
                                    "agent_task",
                                    "researcher",
                                    "Collect constraints, requirements, and user goals from provided context.",
                                );
                                research.next = Some("draft".to_string());
                                research
                            },
                            workflow_step(
                                "draft",
                                "agent_task",
                                "writer",
                                "Write a concise PRD with problem, goals, scope, and milestones.",
                            ),
                        ],
                    },
                    WorkflowConfig {
                        id: "release_notes".to_string(),
                        version: 1,
                        inputs: serde_yaml::Value::Sequence(Vec::new()),
                        limits: None,
                        steps: vec![workflow_step(
                            "compose",
                            "agent_task",
                            "writer",
                            "Write release notes grouped by user impact and breaking changes.",
                        )],
                    },
                ],
            )
        }
    };

    OrchestratorConfig {
        id: id.to_string(),
        selector_agent: selector,
        default_workflow,
        selection_max_retries: 1,
        selector_timeout_seconds: 30,
        agents,
        workflows,
        workflow_orchestration: None,
    }
}

fn default_orchestrator_config(id: &str) -> OrchestratorConfig {
    initial_orchestrator_config(id, "anthropic", "sonnet", SetupWorkflowBundle::Minimal)
}
