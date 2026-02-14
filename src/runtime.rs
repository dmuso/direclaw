use crate::config::Settings;
use crate::orchestrator::{self, FunctionRegistry, RoutedSelectorAction, WorkflowRunStore};
use crate::provider::{self, ProviderError, ProviderKind, ProviderRequest, RunnerBinaries};
use crate::queue::{self, OutgoingMessage, QueuePaths};
use crate::slack;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const QUEUE_MAX_CONCURRENCY: usize = 4;
const QUEUE_MIN_POLL_MS: u64 = 100;
const QUEUE_MAX_POLL_MS: u64 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingDefaults {
    pub queue_poll_interval_secs: u64,
    pub outbound_poll_interval_secs: u64,
}

impl Default for PollingDefaults {
    fn default() -> Self {
        Self {
            queue_poll_interval_secs: 1,
            outbound_poll_interval_secs: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePaths {
    pub root: PathBuf,
}

impl StatePaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn required_directories(&self) -> Vec<PathBuf> {
        vec![
            self.root.join("queue/incoming"),
            self.root.join("queue/processing"),
            self.root.join("queue/outgoing"),
            self.root.join("files"),
            self.root.join("logs"),
            self.root.join("orchestrator/messages"),
            self.root.join("orchestrator/select/incoming"),
            self.root.join("orchestrator/select/processing"),
            self.root.join("orchestrator/select/results"),
            self.root.join("orchestrator/select/logs"),
            self.root.join("orchestrator/diagnostics/incoming"),
            self.root.join("orchestrator/diagnostics/processing"),
            self.root.join("orchestrator/diagnostics/context"),
            self.root.join("orchestrator/diagnostics/results"),
            self.root.join("orchestrator/diagnostics/logs"),
            self.root.join("workflows/runs"),
            self.root.join("channels"),
            self.root.join("daemon"),
        ]
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.with_extension("yaml")
    }

    pub fn daemon_dir(&self) -> PathBuf {
        self.root.join("daemon")
    }

    pub fn supervisor_state_path(&self) -> PathBuf {
        self.daemon_dir().join("runtime.json")
    }

    pub fn supervisor_lock_path(&self) -> PathBuf {
        self.daemon_dir().join("supervisor.lock")
    }

    pub fn stop_signal_path(&self) -> PathBuf {
        self.daemon_dir().join("stop")
    }

    pub fn runtime_log_path(&self) -> PathBuf {
        self.root.join("logs/runtime.log")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("failed to create runtime path {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to resolve home directory for runtime state root")]
    HomeDirectoryUnavailable,
    #[error("failed to read runtime state {path}: {source}")]
    ReadState {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse runtime state {path}: {source}")]
    ParseState {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write runtime state {path}: {source}")]
    WriteState {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("supervisor is already running with pid {pid}")]
    AlreadyRunning { pid: u32 },
    #[error("no running supervisor instance")]
    NotRunning,
    #[error("failed to read lock file {path}: {source}")]
    ReadLock {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write lock file {path}: {source}")]
    WriteLock {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to spawn supervisor process: {0}")]
    Spawn(String),
    #[error("failed to stop supervisor process {pid}; process is still alive")]
    StopFailedAlive { pid: u32 },
}

pub const DEFAULT_STATE_ROOT_DIR: &str = ".direclaw";

pub fn default_state_root_path() -> Result<PathBuf, RuntimeError> {
    let home = std::env::var_os("HOME").ok_or(RuntimeError::HomeDirectoryUnavailable)?;
    Ok(PathBuf::from(home).join(DEFAULT_STATE_ROOT_DIR))
}

pub fn bootstrap_state_root(paths: &StatePaths) -> Result<(), RuntimeError> {
    for path in paths.required_directories() {
        fs::create_dir_all(&path).map_err(|source| RuntimeError::CreateDir {
            path: path.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WorkerKind {
    QueueProcessor,
    Orchestrator,
    ChannelAdapter(String),
    Heartbeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Stopped,
    Running,
    Error,
}

#[derive(Debug, Default)]
pub struct WorkerRegistry {
    workers: HashMap<WorkerKind, WorkerState>,
}

impl WorkerRegistry {
    pub fn register(&mut self, worker: WorkerKind) {
        self.workers.entry(worker).or_insert(WorkerState::Stopped);
    }

    pub fn start(&mut self, worker: &WorkerKind) {
        if let Some(state) = self.workers.get_mut(worker) {
            *state = WorkerState::Running;
        }
    }

    pub fn stop(&mut self, worker: &WorkerKind) {
        if let Some(state) = self.workers.get_mut(worker) {
            *state = WorkerState::Stopped;
        }
    }

    pub fn fail(&mut self, worker: &WorkerKind) {
        if let Some(state) = self.workers.get_mut(worker) {
            *state = WorkerState::Error;
        }
    }

    pub fn state(&self, worker: &WorkerKind) -> Option<WorkerState> {
        self.workers.get(worker).copied()
    }

    pub fn all(&self) -> &HashMap<WorkerKind, WorkerState> {
        &self.workers
    }
}

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

#[derive(Debug, Clone)]
enum WorkerRuntime {
    QueueProcessor,
    OrchestratorDispatcher,
    Slack,
    Heartbeat,
}

#[derive(Debug)]
enum WorkerEvent {
    Started {
        worker_id: String,
        at: i64,
    },
    Heartbeat {
        worker_id: String,
        at: i64,
    },
    Error {
        worker_id: String,
        at: i64,
        message: String,
        fatal: bool,
    },
    Stopped {
        worker_id: String,
        at: i64,
    },
}

#[derive(Debug, Clone)]
struct WorkerSpec {
    id: String,
    runtime: WorkerRuntime,
    interval: Duration,
}

pub fn canonicalize_existing(path: &Path) -> Result<PathBuf, std::io::Error> {
    fs::canonicalize(path)
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

pub fn append_runtime_log(paths: &StatePaths, level: &str, event: &str, message: &str) {
    let path = paths.runtime_log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let payload = Value::Object(Map::from_iter([
        (
            "timestamp".to_string(),
            Value::Number(serde_json::Number::from(now_secs())),
        ),
        ("level".to_string(), Value::String(level.to_string())),
        ("event".to_string(), Value::String(event.to_string())),
        ("message".to_string(), Value::String(message.to_string())),
    ]));

    let Ok(line) = serde_json::to_string(&payload) else {
        return;
    };

    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()));
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

pub fn run_supervisor(state_root: &Path, settings: Settings) -> Result<(), RuntimeError> {
    let paths = StatePaths::new(state_root);
    bootstrap_state_root(&paths)?;

    let stop_path = paths.stop_signal_path();
    if stop_path.exists() {
        let _ = fs::remove_file(&stop_path);
    }

    let specs = build_worker_specs(&settings);
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
            run_worker(
                spec,
                root,
                settings_clone,
                stop_flag,
                tx,
                should_fail,
                slow_shutdown,
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

fn build_worker_specs(settings: &Settings) -> Vec<WorkerSpec> {
    let mut specs = Vec::new();
    specs.push(WorkerSpec {
        id: "queue_processor".to_string(),
        runtime: WorkerRuntime::QueueProcessor,
        interval: Duration::from_millis(250),
    });
    specs.push(WorkerSpec {
        id: "orchestrator_dispatcher".to_string(),
        runtime: WorkerRuntime::OrchestratorDispatcher,
        interval: Duration::from_secs(1),
    });

    if settings.monitoring.heartbeat_interval.unwrap_or(0) > 0 {
        specs.push(WorkerSpec {
            id: "heartbeat".to_string(),
            runtime: WorkerRuntime::Heartbeat,
            interval: Duration::from_secs(settings.monitoring.heartbeat_interval.unwrap_or(3600)),
        });
    }

    for (channel, config) in &settings.channels {
        if !config.enabled {
            continue;
        }
        if channel == "slack" {
            specs.push(WorkerSpec {
                id: format!("channel:{channel}"),
                runtime: WorkerRuntime::Slack,
                interval: Duration::from_secs(2),
            });
        }
    }

    specs
}

fn run_worker(
    spec: WorkerSpec,
    state_root: PathBuf,
    settings: Settings,
    stop: Arc<AtomicBool>,
    events: Sender<WorkerEvent>,
    should_fail: bool,
    slow_shutdown: bool,
) {
    let _ = events.send(WorkerEvent::Started {
        worker_id: spec.id.clone(),
        at: now_secs(),
    });

    if should_fail {
        let _ = events.send(WorkerEvent::Error {
            worker_id: spec.id.clone(),
            at: now_secs(),
            message: "fault injection requested".to_string(),
            fatal: true,
        });
        let _ = events.send(WorkerEvent::Stopped {
            worker_id: spec.id,
            at: now_secs(),
        });
        return;
    }

    if matches!(spec.runtime, WorkerRuntime::QueueProcessor) {
        run_queue_processor_loop(
            spec.id,
            state_root,
            settings,
            stop,
            events,
            slow_shutdown,
            QUEUE_MAX_CONCURRENCY,
        );
        return;
    }

    loop {
        if stop.load(Ordering::Relaxed) {
            if slow_shutdown {
                thread::sleep(Duration::from_secs(6));
            }
            break;
        }

        let tick = match spec.runtime {
            WorkerRuntime::QueueProcessor => Ok(()),
            WorkerRuntime::OrchestratorDispatcher => Ok(()),
            WorkerRuntime::Slack => tick_slack_worker(&state_root, &settings),
            WorkerRuntime::Heartbeat => Ok(()),
        };

        match tick {
            Ok(()) => {
                let _ = events.send(WorkerEvent::Heartbeat {
                    worker_id: spec.id.clone(),
                    at: now_secs(),
                });
            }
            Err(message) => {
                let _ = events.send(WorkerEvent::Error {
                    worker_id: spec.id.clone(),
                    at: now_secs(),
                    message,
                    fatal: false,
                });
            }
        }

        if !sleep_with_stop(&stop, spec.interval) {
            if slow_shutdown {
                thread::sleep(Duration::from_secs(6));
            }
            break;
        }
    }

    let _ = events.send(WorkerEvent::Stopped {
        worker_id: spec.id,
        at: now_secs(),
    });
}

#[derive(Debug)]
struct QueueTaskCompletion {
    key: queue::OrderingKey,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct QueueProcessorLoopConfig {
    slow_shutdown: bool,
    max_concurrency: usize,
    binaries: RunnerBinaries,
}

pub fn recover_processing_queue_entries(state_root: &Path) -> Result<Vec<PathBuf>, String> {
    let queue_paths = QueuePaths::from_state_root(state_root);
    let mut recovered = Vec::new();
    let mut entries = Vec::new();

    for entry in fs::read_dir(&queue_paths.processing).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() {
            entries.push(path);
        }
    }
    entries.sort();

    for (index, processing_path) in entries.into_iter().enumerate() {
        let name = processing_path
            .file_name()
            .and_then(|v| v.to_str())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("message.json");
        let target = queue_paths
            .incoming
            .join(format!("recovered_{index}_{name}"));
        fs::rename(&processing_path, &target).map_err(|e| {
            format!(
                "failed to recover processing file {}: {}",
                processing_path.display(),
                e
            )
        })?;
        recovered.push(target);
    }

    Ok(recovered)
}

pub fn drain_queue_once(
    state_root: &Path,
    settings: &Settings,
    max_concurrency: usize,
) -> Result<usize, String> {
    let binaries = resolve_runner_binaries();
    drain_queue_once_with_binaries(state_root, settings, max_concurrency, &binaries)
}

pub fn drain_queue_once_with_binaries(
    state_root: &Path,
    settings: &Settings,
    max_concurrency: usize,
    binaries: &RunnerBinaries,
) -> Result<usize, String> {
    let queue_paths = QueuePaths::from_state_root(state_root);
    let mut scheduler = queue::PerKeyScheduler::default();
    let (result_tx, result_rx) = mpsc::channel::<QueueTaskCompletion>();
    let mut in_flight = 0usize;
    let mut processed = 0usize;

    while let Some(claimed) = queue::claim_oldest(&queue_paths).map_err(|e| e.to_string())? {
        let key = queue::derive_ordering_key(&claimed.payload);
        scheduler.enqueue(key, claimed);
    }

    loop {
        let available = max_concurrency.saturating_sub(in_flight);
        if available > 0 {
            for scheduled in scheduler.dequeue_runnable(available) {
                let tx = result_tx.clone();
                let root = state_root.to_path_buf();
                let cfg = settings.clone();
                let bins = binaries.clone();
                let _ = thread::spawn(move || {
                    let result = process_claimed_message(&root, &cfg, scheduled.value, &bins).err();
                    let _ = tx.send(QueueTaskCompletion {
                        key: scheduled.key,
                        error: result,
                    });
                });
                in_flight += 1;
            }
        }

        if in_flight == 0 {
            break;
        }

        let completion = match result_rx.recv_timeout(Duration::from_millis(QUEUE_MIN_POLL_MS)) {
            Ok(done) => done,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err("queue worker completion channel disconnected".to_string())
            }
        };
        in_flight = in_flight.saturating_sub(1);
        scheduler.complete(&completion.key);
        if completion.error.is_none() {
            processed += 1;
        } else if let Some(error) = completion.error {
            return Err(error);
        }
    }

    Ok(processed)
}

fn run_queue_processor_loop(
    worker_id: String,
    state_root: PathBuf,
    settings: Settings,
    stop: Arc<AtomicBool>,
    events: Sender<WorkerEvent>,
    slow_shutdown: bool,
    max_concurrency: usize,
) {
    let config = QueueProcessorLoopConfig {
        slow_shutdown,
        max_concurrency,
        binaries: resolve_runner_binaries(),
    };
    run_queue_processor_loop_with_binaries(worker_id, state_root, settings, stop, events, config);
}

fn run_queue_processor_loop_with_binaries(
    worker_id: String,
    state_root: PathBuf,
    settings: Settings,
    stop: Arc<AtomicBool>,
    events: Sender<WorkerEvent>,
    config: QueueProcessorLoopConfig,
) {
    match recover_processing_queue_entries(&state_root) {
        Ok(recovered) => {
            for path in recovered {
                append_runtime_log(
                    &StatePaths::new(&state_root),
                    "info",
                    "queue.recovered",
                    &format!("requeued {}", path.display()),
                );
            }
        }
        Err(error) => {
            let _ = events.send(WorkerEvent::Error {
                worker_id: worker_id.clone(),
                at: now_secs(),
                message: error,
                fatal: false,
            });
        }
    }

    let queue_paths = QueuePaths::from_state_root(&state_root);
    let (result_tx, result_rx) = mpsc::channel::<QueueTaskCompletion>();
    let mut scheduler = queue::PerKeyScheduler::default();
    let mut in_flight = 0usize;
    let mut backoff_ms = QUEUE_MIN_POLL_MS;
    loop {
        let stopping = stop.load(Ordering::Relaxed);

        if !stopping {
            let mut claim_budget = config.max_concurrency.saturating_mul(4);
            while claim_budget > 0 {
                match queue::claim_oldest(&queue_paths) {
                    Ok(Some(claimed)) => {
                        let key = queue::derive_ordering_key(&claimed.payload);
                        scheduler.enqueue(key, claimed);
                    }
                    Ok(None) => break,
                    Err(err) => {
                        let _ = events.send(WorkerEvent::Error {
                            worker_id: worker_id.clone(),
                            at: now_secs(),
                            message: err.to_string(),
                            fatal: false,
                        });
                        break;
                    }
                }
                claim_budget -= 1;
            }
        }

        let available_slots = config.max_concurrency.saturating_sub(in_flight);
        if !stopping && available_slots > 0 {
            for scheduled in scheduler.dequeue_runnable(available_slots) {
                let tx = result_tx.clone();
                let root = state_root.clone();
                let cfg = settings.clone();
                let bins = config.binaries.clone();
                let _ = thread::spawn(move || {
                    let err = process_claimed_message(&root, &cfg, scheduled.value, &bins).err();
                    let _ = tx.send(QueueTaskCompletion {
                        key: scheduled.key,
                        error: err,
                    });
                });
                in_flight += 1;
            }
        }

        while let Ok(done) = result_rx.try_recv() {
            handle_queue_task_completion(&worker_id, &events, &mut scheduler, &mut in_flight, done);
        }

        if stopping {
            if in_flight == 0 {
                for pending in scheduler.drain_pending() {
                    let _ = queue::requeue_failure(&queue_paths, &pending.value);
                }
                if config.slow_shutdown {
                    thread::sleep(Duration::from_secs(6));
                }
                break;
            }
            match result_rx.recv_timeout(Duration::from_millis(QUEUE_MIN_POLL_MS)) {
                Ok(done) => handle_queue_task_completion(
                    &worker_id,
                    &events,
                    &mut scheduler,
                    &mut in_flight,
                    done,
                ),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    in_flight = 0;
                }
            }
            continue;
        }

        if scheduler.pending_len() == 0 && in_flight == 0 {
            let _ = events.send(WorkerEvent::Heartbeat {
                worker_id: worker_id.clone(),
                at: now_secs(),
            });
            if !sleep_with_stop(&stop, Duration::from_millis(backoff_ms)) {
                if config.slow_shutdown {
                    thread::sleep(Duration::from_secs(6));
                }
                break;
            }
            backoff_ms = (backoff_ms.saturating_mul(2)).min(QUEUE_MAX_POLL_MS);
        } else {
            backoff_ms = QUEUE_MIN_POLL_MS;
            thread::sleep(Duration::from_millis(QUEUE_MIN_POLL_MS));
        }
    }

    let _ = events.send(WorkerEvent::Stopped {
        worker_id,
        at: now_secs(),
    });
}

fn handle_queue_task_completion(
    worker_id: &str,
    events: &Sender<WorkerEvent>,
    scheduler: &mut queue::PerKeyScheduler<queue::ClaimedMessage>,
    in_flight: &mut usize,
    done: QueueTaskCompletion,
) {
    *in_flight = in_flight.saturating_sub(1);
    scheduler.complete(&done.key);
    if let Some(message) = done.error {
        let _ = events.send(WorkerEvent::Error {
            worker_id: worker_id.to_string(),
            at: now_secs(),
            message,
            fatal: false,
        });
    } else {
        let _ = events.send(WorkerEvent::Heartbeat {
            worker_id: worker_id.to_string(),
            at: now_secs(),
        });
    }
}

fn process_claimed_message(
    state_root: &Path,
    settings: &Settings,
    claimed: queue::ClaimedMessage,
    binaries: &RunnerBinaries,
) -> Result<(), String> {
    let queue_paths = QueuePaths::from_state_root(state_root);
    let run_store = WorkflowRunStore::new(state_root);
    let functions = FunctionRegistry::with_run_store(
        vec![
            "workflow.status".to_string(),
            "workflow.cancel".to_string(),
            "orchestrator.list".to_string(),
        ],
        run_store,
    );

    let action = orchestrator::process_queued_message(
        state_root,
        settings,
        &claimed.payload,
        now_secs(),
        &BTreeMap::new(),
        &functions,
        |attempt, request, orchestrator_cfg| {
            run_selector_attempt_with_provider(
                state_root,
                settings,
                request,
                orchestrator_cfg,
                attempt,
                binaries,
            )
            .ok()
        },
    )
    .map_err(|e| {
        let _ = queue::requeue_failure(&queue_paths, &claimed);
        e.to_string()
    })?;

    let (response_message, response_agent) = action_to_outbound(&action);
    let outgoing = OutgoingMessage {
        channel: claimed.payload.channel.clone(),
        channel_profile_id: claimed.payload.channel_profile_id.clone(),
        sender: claimed.payload.sender.clone(),
        message: response_message,
        original_message: claimed.payload.message.clone(),
        timestamp: now_secs(),
        message_id: claimed.payload.message_id.clone(),
        agent: response_agent,
        conversation_id: claimed.payload.conversation_id.clone(),
        files: Vec::new(),
        workflow_run_id: claimed.payload.workflow_run_id.clone(),
        workflow_step_id: claimed.payload.workflow_step_id.clone(),
    };
    queue::complete_success(&queue_paths, &claimed, &outgoing).map_err(|e| e.to_string())?;
    Ok(())
}

fn run_selector_attempt_with_provider(
    state_root: &Path,
    settings: &Settings,
    request: &orchestrator::SelectorRequest,
    orchestrator_cfg: &crate::config::OrchestratorConfig,
    attempt: u32,
    binaries: &RunnerBinaries,
) -> Result<String, String> {
    let selector_agent = orchestrator_cfg
        .agents
        .get(&orchestrator_cfg.selector_agent)
        .ok_or_else(|| {
            format!(
                "selector agent `{}` missing from orchestrator config",
                orchestrator_cfg.selector_agent
            )
        })?;
    let provider = ProviderKind::try_from(selector_agent.provider.as_str())
        .map_err(|e| format!("invalid selector provider: {e}"))?;

    let private_workspace = settings
        .resolve_private_workspace(&orchestrator_cfg.id)
        .map_err(|e| e.to_string())?;
    let cwd = match &selector_agent.private_workspace {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => private_workspace.join(path),
        None => private_workspace
            .join("agents")
            .join(&orchestrator_cfg.selector_agent),
    };
    fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;

    let request_json = serde_json::to_string_pretty(request).map_err(|e| e.to_string())?;
    let prompt = format!(
        "You are the workflow selector. Return a strict JSON object only with no prose.\n{}",
        request_json
    );
    let context = format!(
        "orchestratorId={}\nselectorAgent={}\nattempt={attempt}",
        orchestrator_cfg.id, orchestrator_cfg.selector_agent
    );
    let request_id = format!("{}_attempt_{attempt}", request.selector_id);
    let artifacts = provider::write_file_backed_prompt(&cwd, &request_id, &prompt, &context)
        .map_err(|e| e.to_string())?;

    let provider_request = ProviderRequest {
        agent_id: orchestrator_cfg.selector_agent.clone(),
        provider: provider.clone(),
        model: selector_agent.model.clone(),
        cwd: cwd.clone(),
        message: format!(
            "Read [file: {}] and [file: {}]. Return only the selector JSON object.",
            artifacts.prompt_file.display(),
            artifacts
                .context_files
                .first()
                .map(|v| v.display().to_string())
                .unwrap_or_default()
        ),
        prompt_artifacts: artifacts.clone(),
        timeout: Duration::from_secs(30),
        reset_requested: false,
        fresh_on_failure: false,
        env_overrides: BTreeMap::new(),
    };

    match provider::run_provider(&provider_request, binaries) {
        Ok(result) => {
            persist_selector_invocation_log(
                state_root,
                &request.selector_id,
                attempt,
                Some(&result.log),
                None,
            );
            Ok(result.message)
        }
        Err(err) => {
            let log = provider_error_log(&err);
            let error_text = err.to_string();
            persist_selector_invocation_log(
                state_root,
                &request.selector_id,
                attempt,
                log.as_ref(),
                Some(&error_text),
            );
            Err(error_text)
        }
    }
}

fn provider_error_log(err: &ProviderError) -> Option<provider::InvocationLog> {
    match err {
        ProviderError::MissingBinary { log, .. } => Some((**log).clone()),
        ProviderError::NonZeroExit { log, .. } => Some((**log).clone()),
        ProviderError::Timeout { log, .. } => Some((**log).clone()),
        ProviderError::ParseFailure { log, .. } => log.as_ref().map(|v| (**v).clone()),
        ProviderError::UnknownProvider(_)
        | ProviderError::UnsupportedAnthropicModel(_)
        | ProviderError::Io { .. } => None,
    }
}

fn persist_selector_invocation_log(
    state_root: &Path,
    selector_id: &str,
    attempt: u32,
    log: Option<&provider::InvocationLog>,
    error: Option<&str>,
) {
    let path = state_root
        .join("orchestrator/select/logs")
        .join(format!("{selector_id}_attempt_{attempt}.invocation.json"));
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut payload = Map::new();
    payload.insert(
        "selectorId".to_string(),
        Value::String(selector_id.to_string()),
    );
    payload.insert("attempt".to_string(), Value::from(attempt));
    payload.insert("timestamp".to_string(), Value::from(now_secs()));
    payload.insert(
        "status".to_string(),
        Value::String(
            if error.is_some() {
                "failed"
            } else {
                "succeeded"
            }
            .to_string(),
        ),
    );
    if let Some(error) = error {
        payload.insert("error".to_string(), Value::String(error.to_string()));
    }

    if let Some(log) = log {
        payload.insert("agentId".to_string(), Value::String(log.agent_id.clone()));
        payload.insert(
            "provider".to_string(),
            Value::String(log.provider.to_string()),
        );
        payload.insert("model".to_string(), Value::String(log.model.clone()));
        payload.insert(
            "commandForm".to_string(),
            Value::String(log.command_form.clone()),
        );
        payload.insert(
            "workingDirectory".to_string(),
            Value::String(log.working_directory.display().to_string()),
        );
        payload.insert(
            "promptFile".to_string(),
            Value::String(log.prompt_file.display().to_string()),
        );
        payload.insert(
            "contextFiles".to_string(),
            Value::Array(
                log.context_files
                    .iter()
                    .map(|v| Value::String(v.display().to_string()))
                    .collect(),
            ),
        );
        payload.insert(
            "exitCode".to_string(),
            match log.exit_code {
                Some(v) => Value::from(v),
                None => Value::Null,
            },
        );
        payload.insert("timedOut".to_string(), Value::Bool(log.timed_out));
    }

    let encoded = match serde_json::to_vec_pretty(&Value::Object(payload)) {
        Ok(encoded) => encoded,
        Err(_) => return,
    };
    let _ = fs::write(path, encoded);
}

fn resolve_runner_binaries() -> RunnerBinaries {
    RunnerBinaries {
        anthropic: std::env::var("DIRECLAW_PROVIDER_BIN_ANTHROPIC")
            .unwrap_or_else(|_| "claude".to_string()),
        openai: std::env::var("DIRECLAW_PROVIDER_BIN_OPENAI")
            .unwrap_or_else(|_| "codex".to_string()),
    }
}

fn action_to_outbound(action: &RoutedSelectorAction) -> (String, String) {
    match action {
        RoutedSelectorAction::WorkflowStart {
            run_id,
            workflow_id,
        } => (
            format!("workflow started\nrun_id={run_id}\nworkflow_id={workflow_id}"),
            "orchestrator".to_string(),
        ),
        RoutedSelectorAction::WorkflowStatus {
            run_id,
            progress,
            message,
        } => {
            if let Some(progress) = progress {
                (
                    format!(
                        "{message}\nrun_id={}\nstate={}",
                        run_id.clone().unwrap_or_else(|| "none".to_string()),
                        progress.state,
                    ),
                    "orchestrator".to_string(),
                )
            } else {
                (message.clone(), "orchestrator".to_string())
            }
        }
        RoutedSelectorAction::DiagnosticsInvestigate { findings, .. } => {
            (findings.clone(), "diagnostics".to_string())
        }
        RoutedSelectorAction::CommandInvoke { result } => {
            let rendered = serde_json::to_string_pretty(result)
                .unwrap_or_else(|_| "command completed".to_string());
            (rendered, "command".to_string())
        }
    }
}

fn tick_slack_worker(state_root: &Path, settings: &Settings) -> Result<(), String> {
    slack::sync_once(state_root, settings)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn sleep_with_stop(stop: &AtomicBool, total: Duration) -> bool {
    let mut remaining = total;
    while remaining > Duration::from_millis(0) {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let step = remaining.min(Duration::from_millis(200));
        thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
    !stop.load(Ordering::Relaxed)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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

fn atomic_write_file(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("path has no parent"))?;
    let tmp_name = format!(
        ".{}.tmp-{}-{}",
        path.file_name().and_then(|v| v.to_str()).unwrap_or("state"),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let tmp_path = parent.join(tmp_name);

    {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp_path)?;
        file.write_all(content)?;
        file.sync_all()?;
    }

    fs::rename(&tmp_path, path)?;
    sync_parent_dir(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> std::io::Result<()> {
    fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{IncomingMessage, QueuePaths};
    use std::sync::mpsc::RecvTimeoutError;
    use tempfile::tempdir;

    #[test]
    fn polling_defaults_are_one_second() {
        let defaults = PollingDefaults::default();
        assert_eq!(defaults.queue_poll_interval_secs, 1);
        assert_eq!(defaults.outbound_poll_interval_secs, 1);
    }

    #[test]
    fn bootstrap_creates_required_directories() {
        let dir = tempdir().expect("temp dir");
        let paths = StatePaths::new(dir.path().join("state"));
        bootstrap_state_root(&paths).expect("bootstrap succeeds");

        for required in paths.required_directories() {
            assert!(
                required.is_dir(),
                "missing directory: {}",
                required.display()
            );
        }
    }

    #[test]
    fn settings_file_uses_global_direclaw_yaml_path() {
        let paths = StatePaths::new("/tmp/.direclaw");
        assert_eq!(paths.settings_file(), PathBuf::from("/tmp/.direclaw.yaml"));
    }

    #[test]
    fn default_state_root_path_uses_home_direclaw() {
        let dir = tempdir().expect("temp dir");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());

        let root = default_state_root_path().expect("resolve state root");
        assert_eq!(root, dir.path().join(".direclaw"));

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn worker_registry_tracks_independent_lifecycle() {
        let mut registry = WorkerRegistry::default();
        let queue = WorkerKind::QueueProcessor;
        let orchestrator = WorkerKind::Orchestrator;
        let slack = WorkerKind::ChannelAdapter("slack".to_string());
        let heartbeat = WorkerKind::Heartbeat;

        registry.register(queue.clone());
        registry.register(orchestrator.clone());
        registry.register(slack.clone());
        registry.register(heartbeat.clone());

        registry.start(&queue);
        registry.start(&slack);

        assert_eq!(registry.state(&queue), Some(WorkerState::Running));
        assert_eq!(registry.state(&orchestrator), Some(WorkerState::Stopped));
        assert_eq!(registry.state(&slack), Some(WorkerState::Running));
        assert_eq!(registry.state(&heartbeat), Some(WorkerState::Stopped));

        registry.fail(&slack);
        assert_eq!(registry.state(&slack), Some(WorkerState::Error));
        registry.stop(&slack);
        assert_eq!(registry.state(&slack), Some(WorkerState::Stopped));
    }

    #[test]
    fn stale_state_is_cleaned_when_pid_not_running() {
        let dir = tempdir().expect("tempdir");
        let paths = StatePaths::new(dir.path().join(".direclaw"));
        bootstrap_state_root(&paths).expect("bootstrap");

        let stale = SupervisorState {
            running: true,
            pid: Some(999_999),
            started_at: Some(1),
            stopped_at: None,
            workers: BTreeMap::new(),
            last_error: None,
        };
        save_supervisor_state(&paths, &stale).expect("save stale");
        fs::write(paths.supervisor_lock_path(), "999999").expect("lock");

        let ownership = supervisor_ownership_state(&paths).expect("ownership");
        assert_eq!(ownership, OwnershipState::Stale);

        cleanup_stale_supervisor(&paths).expect("cleanup stale");
        assert_eq!(
            supervisor_ownership_state(&paths).expect("ownership after"),
            OwnershipState::NotRunning
        );

        let cleaned = load_supervisor_state(&paths).expect("load cleaned");
        assert!(!cleaned.running);
        assert!(cleaned.pid.is_none());
    }

    #[test]
    fn reserve_start_lock_is_exclusive_until_cleared() {
        let dir = tempdir().expect("tempdir");
        let paths = StatePaths::new(dir.path().join(".direclaw"));
        bootstrap_state_root(&paths).expect("bootstrap");

        reserve_start_lock(&paths).expect("reserve");
        let second = reserve_start_lock(&paths).expect_err("second reserve must fail");
        assert!(second.to_string().contains("failed to write lock file"));

        clear_start_lock(&paths);
        reserve_start_lock(&paths).expect("reserve after clear");
    }

    fn sample_incoming(message_id: &str) -> IncomingMessage {
        IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("eng".to_string()),
            sender: "Dana".to_string(),
            sender_id: "U42".to_string(),
            message: "help".to_string(),
            timestamp: 100,
            message_id: message_id.to_string(),
            conversation_id: Some("thread-1".to_string()),
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        }
    }

    fn write_settings_and_orchestrator(
        workspace_root: &Path,
        orchestrator_workspace: &Path,
    ) -> Settings {
        fs::create_dir_all(orchestrator_workspace).expect("orchestrator workspace");
        fs::write(
            orchestrator_workspace.join("orchestrator.yaml"),
            r#"
id: eng_orchestrator
selector_agent: router
default_workflow: triage
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.2
workflows:
  - id: triage
    version: 1
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
"#,
        )
        .expect("write orchestrator");

        serde_yaml::from_str(&format!(
            r#"
workspace_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  eng_orchestrator:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles:
  eng:
    channel: slack
    orchestrator_id: eng_orchestrator
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
            workspace = workspace_root.display(),
            orchestrator_workspace = orchestrator_workspace.display()
        ))
        .expect("settings")
    }

    #[test]
    fn queue_worker_stop_while_in_flight_completes_and_terminates() {
        let dir = tempdir().expect("tempdir");
        let state_root = dir.path().join(".direclaw");
        let paths = StatePaths::new(&state_root);
        bootstrap_state_root(&paths).expect("bootstrap");
        let queue = QueuePaths::from_state_root(&state_root);

        let settings = write_settings_and_orchestrator(dir.path(), &dir.path().join("orch"));
        fs::write(
            queue.incoming.join("msg-stop.json"),
            serde_json::to_vec(&sample_incoming("msg-stop")).expect("serialize"),
        )
        .expect("write incoming");

        let claude = dir.path().join("claude-sleep");
        fs::write(
            &claude,
            "#!/bin/sh\nsleep 1\necho '{\"selectorId\":\"sel-msg-stop\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
        )
        .expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&claude).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&claude, perms).expect("chmod");
        }

        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<WorkerEvent>();
        let handle = thread::spawn({
            let stop = stop.clone();
            let settings = settings.clone();
            let state_root = state_root.clone();
            let binaries = RunnerBinaries {
                anthropic: claude.display().to_string(),
                openai: "unused".to_string(),
            };
            let config = QueueProcessorLoopConfig {
                slow_shutdown: false,
                max_concurrency: 1,
                binaries,
            };
            move || {
                run_queue_processor_loop_with_binaries(
                    "queue_processor".to_string(),
                    state_root,
                    settings,
                    stop,
                    tx,
                    config,
                )
            }
        });

        let start = std::time::Instant::now();
        while fs::read_dir(&queue.processing)
            .expect("processing dir")
            .next()
            .is_none()
        {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "queue task never moved to processing"
            );
            thread::sleep(Duration::from_millis(25));
        }
        stop.store(true, Ordering::Relaxed);

        loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(WorkerEvent::Stopped { .. }) => break,
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => panic!("queue worker did not terminate"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("queue worker event channel disconnected before stop event")
                }
            }
        }
        handle.join().expect("join queue worker");

        assert!(
            fs::read_dir(&queue.processing)
                .expect("processing dir")
                .next()
                .is_none(),
            "processing directory should be empty after stop"
        );
    }
}
