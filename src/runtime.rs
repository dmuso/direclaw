use crate::config::Settings;
use crate::slack;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod logging;
pub mod queue_worker;
pub mod recovery;
pub mod state_paths;
pub mod supervisor;
pub mod worker_registry;

pub use logging::append_runtime_log;
pub use queue_worker::{drain_queue_once, drain_queue_once_with_binaries};
pub use recovery::recover_processing_queue_entries;
pub use state_paths::{
    bootstrap_state_root, default_state_root_path, StatePaths, DEFAULT_STATE_ROOT_DIR,
};
pub use supervisor::{
    cleanup_stale_supervisor, clear_start_lock, is_process_alive, load_supervisor_state,
    reserve_start_lock, save_supervisor_state, signal_stop, spawn_supervisor_process,
    stop_active_supervisor, supervisor_ownership_state, write_supervisor_lock_pid, OwnershipState,
    StopResult, SupervisorState, WorkerHealth,
};
pub use worker_registry::{WorkerKind, WorkerRegistry, WorkerState};

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

#[derive(Debug, Clone)]
enum WorkerRuntime {
    QueueProcessor,
    OrchestratorDispatcher,
    Slack,
    Heartbeat,
}

#[derive(Debug)]
pub(crate) enum WorkerEvent {
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

    if matches!(spec.runtime, WorkerRuntime::Slack) {
        if let Err(err) = slack::validate_startup_credentials(&settings) {
            let _ = events.send(WorkerEvent::Error {
                worker_id: spec.id.clone(),
                at: now_secs(),
                message: err.to_string(),
                fatal: true,
            });
            let _ = events.send(WorkerEvent::Stopped {
                worker_id: spec.id,
                at: now_secs(),
            });
            return;
        }
    }

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
        queue_worker::run_queue_processor_loop(
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
    use crate::provider::RunnerBinaries;
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
    fn settings_file_uses_global_direclaw_config_yaml_path() {
        let paths = StatePaths::new("/tmp/.direclaw");
        assert_eq!(
            paths.settings_file(),
            PathBuf::from("/tmp/.direclaw/config.yaml")
        );
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
workspaces_path: {workspace}
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
            let config = queue_worker::QueueProcessorLoopConfig {
                slow_shutdown: false,
                max_concurrency: 1,
                binaries,
            };
            move || {
                queue_worker::run_queue_processor_loop_with_binaries(
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
