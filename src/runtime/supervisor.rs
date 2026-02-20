use super::{
    append_runtime_log, atomic_write_file, bootstrap_state_root, channel_worker, now_secs,
    ownership_lock, queue_worker, RuntimeError, StatePaths, WorkerEvent, WorkerState,
};
use crate::channels::slack;
use crate::runtime::worker_registry::apply_worker_event;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub use super::worker_registry::WorkerHealth;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SupervisorState {
    pub running: bool,
    pub pid: Option<u32>,
    pub started_at: Option<i64>,
    pub stopped_at: Option<i64>,
    pub workers: BTreeMap<String, WorkerHealth>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub slack_profiles: Vec<slack::SlackProfileCredentialHealth>,
}

pub use super::ownership_lock::{
    cleanup_stale_supervisor, clear_start_lock, is_process_alive, reserve_start_lock, signal_stop,
    spawn_supervisor_process, stop_active_supervisor, supervisor_ownership_state,
    write_supervisor_lock_pid, OwnershipState, StopResult,
};

pub fn run_supervisor(
    state_root: &Path,
    settings: crate::config::Settings,
) -> Result<(), RuntimeError> {
    let paths = StatePaths::new(state_root);
    bootstrap_state_root(&paths)?;

    let stop_path = paths.stop_signal_path();
    if stop_path.exists() {
        let _ = fs::remove_file(&stop_path);
    }

    let specs = channel_worker::build_worker_specs(&settings);
    let mut state = SupervisorState {
        running: true,
        pid: Some(std::process::id()),
        started_at: Some(now_secs()),
        stopped_at: None,
        workers: BTreeMap::new(),
        last_error: None,
        slack_profiles: slack::profile_credential_health(&settings),
    };

    for spec in &specs {
        state
            .workers
            .insert(spec.id.clone(), WorkerHealth::default());
    }
    save_supervisor_state(&paths, &state)?;
    append_runtime_log(
        &paths,
        "info",
        "supervisor.started",
        &format!("pid={} workers={}", std::process::id(), specs.len()),
    );

    let fail_worker = std::env::var("DIRECLAW_FAIL_WORKER").ok();
    let slow_shutdown_worker = std::env::var("DIRECLAW_SLOW_SHUTDOWN_WORKER").ok();
    let stop = Arc::new(AtomicBool::new(false));
    let (events_tx, events_rx) = mpsc::channel::<WorkerEvent>();
    let mut handles = Vec::new();
    let mut active = BTreeSet::new();

    for spec in specs {
        active.insert(spec.id.clone());
        let tx = events_tx.clone();
        let stop_flag = stop.clone();
        let root = paths.root.clone();
        let settings_clone = settings.clone();
        let should_fail = fail_worker
            .as_ref()
            .map(|id| id == &spec.id)
            .unwrap_or(false);
        let slow_shutdown = slow_shutdown_worker
            .as_ref()
            .map(|id| id == &spec.id)
            .unwrap_or(false);

        handles.push(thread::spawn(move || {
            channel_worker::run_worker(
                spec,
                channel_worker::WorkerRunContext {
                    state_root: root,
                    settings: settings_clone,
                    stop: stop_flag,
                    events: tx,
                    should_fail,
                    slow_shutdown,
                    queue_max_concurrency: queue_worker::QUEUE_MAX_CONCURRENCY,
                },
            )
        }));
    }
    drop(events_tx);

    while !stop.load(Ordering::Relaxed) {
        if paths.stop_signal_path().exists() {
            stop.store(true, Ordering::Relaxed);
            append_runtime_log(
                &paths,
                "info",
                "supervisor.stop.signal",
                "stop file detected",
            );
        }

        match events_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(event) => handle_worker_event(&paths, &mut state, &mut active, event),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let shutdown_timeout = shutdown_wait_timeout();
    let deadline = std::time::Instant::now() + shutdown_timeout;
    while !active.is_empty() && std::time::Instant::now() < deadline {
        match events_rx.recv_timeout(Duration::from_millis(25)) {
            Ok(event) => handle_worker_event(&paths, &mut state, &mut active, event),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    if !active.is_empty() {
        let message = format!(
            "shutdown timeout waiting for workers: {}",
            active.iter().cloned().collect::<Vec<_>>().join(",")
        );
        state.last_error = Some(message.clone());
        for worker_id in &active {
            if let Some(worker) = state.workers.get_mut(worker_id) {
                worker.state = WorkerState::Error;
                worker.last_error = Some("shutdown timeout".to_string());
            }
        }
        append_runtime_log(&paths, "warn", "supervisor.shutdown.timeout", &message);
    }

    for handle in handles {
        let _ = handle.join();
    }

    state.running = false;
    state.pid = None;
    state.stopped_at = Some(now_secs());
    save_supervisor_state(&paths, &state)?;

    ownership_lock::clear_start_lock(&paths);
    let _ = fs::remove_file(paths.stop_signal_path());
    append_runtime_log(
        &paths,
        "info",
        "supervisor.stopped",
        "runtime stopped cleanly",
    );
    Ok(())
}

fn shutdown_wait_timeout() -> Duration {
    if let Some(milliseconds) = std::env::var("DIRECLAW_SHUTDOWN_TIMEOUT_MILLISECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
    {
        return Duration::from_millis(milliseconds);
    }
    let seconds = std::env::var("DIRECLAW_SHUTDOWN_TIMEOUT_SECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(5);
    Duration::from_secs(seconds)
}

pub fn load_supervisor_state(paths: &StatePaths) -> Result<SupervisorState, RuntimeError> {
    let path = paths.supervisor_state_path();
    if !path.exists() {
        return Ok(SupervisorState::default());
    }
    let raw = fs::read_to_string(&path).map_err(|source| RuntimeError::ReadState {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|source| RuntimeError::ParseState {
        path: path.display().to_string(),
        source,
    })
}

pub fn save_supervisor_state(
    paths: &StatePaths,
    state: &SupervisorState,
) -> Result<(), RuntimeError> {
    let path = paths.supervisor_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| RuntimeError::CreateDir {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let encoded = serde_json::to_vec_pretty(state).map_err(|source| RuntimeError::ParseState {
        path: path.display().to_string(),
        source,
    })?;
    atomic_write_file(&path, &encoded).map_err(|source| RuntimeError::WriteState {
        path: path.display().to_string(),
        source,
    })
}

fn handle_worker_event(
    paths: &StatePaths,
    state: &mut SupervisorState,
    active: &mut BTreeSet<String>,
    event: WorkerEvent,
) {
    if let Some(log) = apply_worker_event(&mut state.workers, active, event) {
        append_runtime_log(paths, log.level, log.event, &log.message);
    }

    let _ = save_supervisor_state(paths, state);
}
