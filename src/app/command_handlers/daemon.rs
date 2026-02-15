use crate::app::command_handlers::auth::{render_auth_sync_result, sync_auth_sources};
use crate::app::command_support::{ensure_runtime_root, load_settings};
use crate::channels::slack;
use crate::runtime::{
    append_runtime_log, cleanup_stale_supervisor, load_supervisor_state, reserve_start_lock,
    run_supervisor, save_supervisor_state, spawn_supervisor_process, stop_active_supervisor,
    supervisor_ownership_state, OwnershipState, SupervisorState, WorkerHealth, WorkerState,
};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn cmd_start() -> Result<String, String> {
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

pub fn cmd_stop() -> Result<String, String> {
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

pub fn cmd_restart() -> Result<String, String> {
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

fn slack_profile_status_lines(
    settings: &crate::config::Settings,
    state: &SupervisorState,
) -> Vec<String> {
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

pub fn cmd_status() -> Result<String, String> {
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

pub fn cmd_logs() -> Result<String, String> {
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

pub fn cmd_supervisor(args: &[String]) -> Result<String, String> {
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

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
