use super::{
    append_runtime_log, atomic_write_file, bootstrap_state_root, channel_worker, now_secs,
    RuntimeError, StatePaths, WorkerEvent, WorkerState, QUEUE_MAX_CONCURRENCY,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerHealth {
    pub state: WorkerState,
    pub last_heartbeat: Option<i64>,
    pub last_error: Option<String>,
}

impl Default for WorkerHealth {
    fn default() -> Self {
        Self {
            state: WorkerState::Stopped,
            last_heartbeat: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SupervisorState {
    pub running: bool,
    pub pid: Option<u32>,
    pub started_at: Option<i64>,
    pub stopped_at: Option<i64>,
    pub workers: BTreeMap<String, WorkerHealth>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipState {
    NotRunning,
    Running { pid: u32 },
    Stale,
}

#[derive(Debug, Clone)]
pub struct StopResult {
    pub pid: u32,
    pub forced: bool,
}

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
                    queue_max_concurrency: QUEUE_MAX_CONCURRENCY,
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

        match events_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(event) => apply_worker_event(&paths, &mut state, &mut active, event),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !active.is_empty() && std::time::Instant::now() < deadline {
        match events_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => apply_worker_event(&paths, &mut state, &mut active, event),
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

    clear_start_lock(&paths);
    let _ = fs::remove_file(paths.stop_signal_path());
    append_runtime_log(
        &paths,
        "info",
        "supervisor.stopped",
        "runtime stopped cleanly",
    );
    Ok(())
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

pub fn supervisor_ownership_state(paths: &StatePaths) -> Result<OwnershipState, RuntimeError> {
    let state = load_supervisor_state(paths)?;
    if let Some(pid) = state.pid {
        if state.running && is_process_alive(pid) {
            return Ok(OwnershipState::Running { pid });
        }
    }

    if let Some(pid) = read_lock_pid(paths)? {
        if is_process_alive(pid) {
            return Ok(OwnershipState::Running { pid });
        }
        return Ok(OwnershipState::Stale);
    }

    if state.running || state.pid.is_some() {
        return Ok(OwnershipState::Stale);
    }

    Ok(OwnershipState::NotRunning)
}

pub fn cleanup_stale_supervisor(paths: &StatePaths) -> Result<(), RuntimeError> {
    let lock = paths.supervisor_lock_path();
    if lock.exists() {
        let _ = fs::remove_file(&lock);
    }
    let stop = paths.stop_signal_path();
    if stop.exists() {
        let _ = fs::remove_file(&stop);
    }
    let mut state = load_supervisor_state(paths)?;
    state.running = false;
    state.pid = None;
    state.stopped_at = Some(now_secs());
    save_supervisor_state(paths, &state)
}

pub fn reserve_start_lock(paths: &StatePaths) -> Result<(), RuntimeError> {
    let path = paths.supervisor_lock_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| RuntimeError::CreateDir {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .and_then(|mut file| file.write_all(std::process::id().to_string().as_bytes()))
        .map_err(|source| RuntimeError::WriteLock {
            path: path.display().to_string(),
            source,
        })
}

pub fn write_supervisor_lock_pid(paths: &StatePaths, pid: u32) -> Result<(), RuntimeError> {
    let path = paths.supervisor_lock_path();
    atomic_write_file(&path, pid.to_string().as_bytes()).map_err(|source| RuntimeError::WriteLock {
        path: path.display().to_string(),
        source,
    })
}

pub fn clear_start_lock(paths: &StatePaths) {
    let _ = fs::remove_file(paths.supervisor_lock_path());
}

pub fn spawn_supervisor_process(state_root: &Path) -> Result<u32, RuntimeError> {
    let exe = std::env::current_exe().map_err(|e| RuntimeError::Spawn(e.to_string()))?;
    let child = std::process::Command::new(exe)
        .arg("__supervisor")
        .arg("--state-root")
        .arg(state_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| RuntimeError::Spawn(e.to_string()))?;
    Ok(child.id())
}

pub fn signal_stop(paths: &StatePaths) -> Result<(), RuntimeError> {
    let path = paths.stop_signal_path();
    fs::write(&path, b"stop").map_err(|source| RuntimeError::WriteState {
        path: path.display().to_string(),
        source,
    })
}

pub fn stop_active_supervisor(
    paths: &StatePaths,
    timeout: Duration,
) -> Result<StopResult, RuntimeError> {
    let pid = match supervisor_ownership_state(paths)? {
        OwnershipState::Running { pid } => pid,
        OwnershipState::Stale => {
            cleanup_stale_supervisor(paths)?;
            return Err(RuntimeError::NotRunning);
        }
        OwnershipState::NotRunning => return Err(RuntimeError::NotRunning),
    };

    signal_stop(paths)?;
    append_runtime_log(
        paths,
        "info",
        "supervisor.stop.requested",
        &format!("pid={pid}"),
    );

    let start = std::time::Instant::now();
    while is_process_alive(pid) && start.elapsed() < timeout {
        thread::sleep(Duration::from_millis(100));
    }

    let mut forced = false;
    if is_process_alive(pid) {
        send_signal(pid, "-TERM");
        let sigterm_start = std::time::Instant::now();
        while is_process_alive(pid) && sigterm_start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(100));
        }
    }

    if is_process_alive(pid) {
        forced = true;
        append_runtime_log(
            paths,
            "warn",
            "supervisor.stop.force_kill",
            &format!("pid={pid}"),
        );
        send_signal(pid, "-KILL");
        let sigkill_start = std::time::Instant::now();
        while is_process_alive(pid) && sigkill_start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(100));
        }
    }

    if is_process_alive(pid) {
        append_runtime_log(
            paths,
            "error",
            "supervisor.stop.failed",
            &format!("pid={pid} remained alive after TERM/KILL"),
        );
        return Err(RuntimeError::StopFailedAlive { pid });
    }

    cleanup_stale_supervisor(paths)?;
    Ok(StopResult { pid, forced })
}

fn read_lock_pid(paths: &StatePaths) -> Result<Option<u32>, RuntimeError> {
    let path = paths.supervisor_lock_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|source| RuntimeError::ReadLock {
        path: path.display().to_string(),
        source,
    })?;
    let parsed = raw.trim().parse::<u32>().ok();
    Ok(parsed)
}

pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(unix)]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        false
    }
}

fn send_signal(pid: u32, signal: &str) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .arg(signal)
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    #[cfg(not(unix))]
    {
        let _ = (pid, signal);
    }
}

fn apply_worker_event(
    paths: &StatePaths,
    state: &mut SupervisorState,
    active: &mut BTreeSet<String>,
    event: WorkerEvent,
) {
    match event {
        WorkerEvent::Started { worker_id, at } => {
            let entry = state.workers.entry(worker_id.clone()).or_default();
            entry.state = WorkerState::Running;
            entry.last_heartbeat = Some(at);
            append_runtime_log(paths, "info", "worker.started", &worker_id);
        }
        WorkerEvent::Heartbeat { worker_id, at } => {
            let entry = state.workers.entry(worker_id).or_default();
            if entry.state != WorkerState::Error {
                entry.state = WorkerState::Running;
            }
            entry.last_heartbeat = Some(at);
        }
        WorkerEvent::Error {
            worker_id,
            at,
            message,
            fatal,
        } => {
            let entry = state.workers.entry(worker_id.clone()).or_default();
            entry.state = WorkerState::Error;
            entry.last_heartbeat = Some(at);
            entry.last_error = Some(message.clone());
            append_runtime_log(
                paths,
                if fatal { "error" } else { "warn" },
                "worker.error",
                &format!("{}: {}", worker_id, message),
            );
        }
        WorkerEvent::Stopped { worker_id, at } => {
            let entry = state.workers.entry(worker_id.clone()).or_default();
            if entry.state != WorkerState::Error {
                entry.state = WorkerState::Stopped;
            }
            entry.last_heartbeat = Some(at);
            active.remove(&worker_id);
            append_runtime_log(paths, "info", "worker.stopped", &worker_id);
        }
    }

    let _ = save_supervisor_state(paths, state);
}
