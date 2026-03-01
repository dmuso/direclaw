#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::thread;
#[cfg(test)]
use std::time::Duration;

pub mod channel_worker;
pub mod heartbeat_worker;
pub mod logging;
pub mod memory_worker;
pub mod ownership_lock;
pub mod queue_worker;
pub mod recovery;
pub mod scheduler_worker;
pub mod state_paths;
pub mod supervisor;
pub mod worker_registry;

pub use crate::shared::errors::RuntimeError;
pub(crate) use crate::shared::fs_atomic::atomic_write_file;
pub use crate::shared::fs_atomic::canonicalize_existing;
pub(crate) use crate::shared::time::now_secs;
pub use channel_worker::PollingDefaults;
pub use logging::append_runtime_log;
pub use memory_worker::{bootstrap_memory_runtime_paths, tick_memory_worker};
pub use ownership_lock::{
    cleanup_stale_supervisor, clear_start_lock, is_process_alive, reserve_start_lock, signal_stop,
    spawn_supervisor_process, stop_active_supervisor, supervisor_ownership_state,
    write_supervisor_lock_pid, OwnershipState, StopResult,
};
pub use queue_worker::{drain_queue_once, drain_queue_once_with_binaries};
pub use queue_worker::{queue_polling_defaults, QueuePollingDefaults};
pub use recovery::recover_processing_queue_entries;
pub use state_paths::{
    bootstrap_state_root, default_state_root_path, StatePaths, DEFAULT_STATE_ROOT_DIR,
};
pub use supervisor::{
    load_supervisor_state, run_supervisor, save_supervisor_state, SupervisorState, WorkerHealth,
};
pub use worker_registry::{WorkerEvent, WorkerKind, WorkerRegistry, WorkerState};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::provider::RunnerBinaries;
    use crate::queue::{IncomingMessage, QueuePaths};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::RecvTimeoutError;
    use std::sync::{mpsc, Arc, Mutex};
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
        let _guard = ENV_LOCK.lock().expect("env lock");
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
        let memory = WorkerKind::Memory;

        registry.register(queue.clone());
        registry.register(orchestrator.clone());
        registry.register(slack.clone());
        registry.register(heartbeat.clone());
        registry.register(memory.clone());

        registry.start(&queue);
        registry.start(&slack);

        assert_eq!(registry.state(&queue), Some(WorkerState::Running));
        assert_eq!(registry.state(&orchestrator), Some(WorkerState::Stopped));
        assert_eq!(registry.state(&slack), Some(WorkerState::Running));
        assert_eq!(registry.state(&heartbeat), Some(WorkerState::Stopped));
        assert_eq!(registry.state(&memory), Some(WorkerState::Stopped));

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
            slack_profiles: Vec::new(),
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

    #[test]
    fn channel_worker_builds_specs_for_enabled_slack_and_heartbeat() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring:
  heartbeat_interval: 30
channels:
  slack:
    enabled: true
"#,
        )
        .expect("parse settings");

        let specs = channel_worker::build_worker_specs(&settings);
        let ids = specs
            .iter()
            .map(|spec| spec.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "queue_processor",
                "orchestrator_dispatcher",
                "memory_worker",
                "scheduler",
                "heartbeat",
                "channel:slack-socket"
            ]
        );

        let memory_worker = specs
            .iter()
            .find(|spec| spec.id == "memory_worker")
            .expect("memory worker spec");
        assert_eq!(memory_worker.interval, Duration::from_secs(30));
    }

    #[test]
    fn memory_worker_interval_uses_memory_config_override() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
memory:
  worker_interval_seconds: 7
"#,
        )
        .expect("parse settings");

        let specs = channel_worker::build_worker_specs(&settings);
        let memory_worker = specs
            .iter()
            .find(|spec| spec.id == "memory_worker")
            .expect("memory worker spec");
        assert_eq!(memory_worker.interval, Duration::from_secs(7));
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
            is_direct: false,
            is_thread_reply: false,
            is_mentioned: false,
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
    model: gpt-5.3-codex-spark
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

        let settings = write_settings_and_orchestrator(dir.path(), &dir.path().join("orch"));
        let queue = QueuePaths::from_state_root(
            &settings
                .resolve_channel_profile_runtime_root("eng")
                .expect("runtime root"),
        );
        fs::create_dir_all(&queue.incoming).expect("incoming dir");
        fs::create_dir_all(&queue.processing).expect("processing dir");
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
