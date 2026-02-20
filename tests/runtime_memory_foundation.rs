use direclaw::runtime::{load_supervisor_state, run_supervisor, StatePaths, WorkerState};
use std::fs;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn settings_yaml(workspace: &std::path::Path, memory_enabled: bool) -> String {
    format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  alpha:
    private_workspace: {workspace}/alpha
    shared_access: []
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
memory:
  enabled: {memory_enabled}
  worker_interval_seconds: 30
  bulletin_mode: every_message
  retrieval:
    top_n: 20
    rrf_k: 60
  ingest:
    enabled: true
    max_file_size_mb: 25
  scope:
    cross_orchestrator: false
"#,
        workspace = workspace.display(),
        memory_enabled = memory_enabled
    )
}

fn wait_for_worker_state(
    root: &std::path::Path,
    worker_id: &str,
    expected: WorkerState,
    timeout: Duration,
) {
    let start = Instant::now();
    loop {
        let state = load_supervisor_state(&StatePaths::new(root)).expect("load state");
        if state
            .workers
            .get(worker_id)
            .map(|worker| worker.state == expected)
            .unwrap_or(false)
        {
            return;
        }
        assert!(
            start.elapsed() < timeout,
            "timed out waiting for worker `{worker_id}` to reach state `{expected:?}`"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn run_memory_worker_registration_case(memory_enabled: bool) {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join("state");
    let workspace = dir.path().join("workspaces");
    fs::create_dir_all(workspace.join("alpha")).expect("workspace");

    let settings =
        serde_yaml::from_str(&settings_yaml(&workspace, memory_enabled)).expect("settings");
    let root = state_root.join(if memory_enabled {
        "enabled"
    } else {
        "disabled"
    });
    fs::create_dir_all(&root).expect("state root");
    let stop_path = StatePaths::new(&root).stop_signal_path();

    let handle = thread::spawn({
        let root = root.clone();
        move || run_supervisor(&root, settings)
    });

    wait_for_worker_state(
        &root,
        "orchestrator_dispatcher",
        WorkerState::Running,
        Duration::from_secs(2),
    );
    fs::write(&stop_path, "stop").expect("stop");
    handle.join().expect("join").expect("supervisor");

    let state = load_supervisor_state(&StatePaths::new(&root)).expect("load state");
    let has_memory_worker = state.workers.contains_key("memory_worker");
    assert_eq!(has_memory_worker, memory_enabled);

    if memory_enabled {
        let runtime_root = workspace.join("alpha");
        let memory_root = runtime_root.join("memory");
        let ingest = memory_root.join("ingest");
        let processed = ingest.join("processed");
        let rejected = ingest.join("rejected");
        let bulletins = memory_root.join("bulletins");
        let orchestrator_log = runtime_root.join("logs/orchestrator.log");

        assert!(ingest.is_dir(), "missing {}", ingest.display());
        assert!(processed.is_dir(), "missing {}", processed.display());
        assert!(rejected.is_dir(), "missing {}", rejected.display());
        assert!(bulletins.is_dir(), "missing {}", bulletins.display());
        assert!(
            orchestrator_log.is_file(),
            "missing {}",
            orchestrator_log.display()
        );

        let log_body = fs::read_to_string(&orchestrator_log).expect("read orchestrator log");
        assert!(
            log_body.contains("\"event\":\"memory.worker.bootstrap_complete\""),
            "expected bootstrap line in orchestrator log"
        );
    }
}

#[test]
fn supervisor_registers_memory_worker_when_memory_enabled() {
    run_memory_worker_registration_case(true);
}

#[test]
fn supervisor_omits_memory_worker_when_memory_disabled() {
    run_memory_worker_registration_case(false);
}

#[test]
fn supervisor_surfaces_memory_worker_startup_failures_in_worker_health() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join("state");
    let workspace = dir.path().join("workspaces");
    let orch_root = workspace.join("alpha");
    fs::create_dir_all(&orch_root).expect("workspace");
    fs::write(orch_root.join("memory"), "not a directory").expect("memory blocker file");

    let settings = serde_yaml::from_str(&settings_yaml(&workspace, true)).expect("settings");
    let stop_path = StatePaths::new(&state_root).stop_signal_path();

    let handle = thread::spawn({
        let state_root = state_root.clone();
        move || run_supervisor(&state_root, settings)
    });

    wait_for_worker_state(
        &state_root,
        "memory_worker",
        WorkerState::Error,
        Duration::from_secs(2),
    );
    fs::write(&stop_path, "stop").expect("stop");
    handle.join().expect("join").expect("supervisor");

    let state = load_supervisor_state(&StatePaths::new(&state_root)).expect("load state");
    let worker = state
        .workers
        .get("memory_worker")
        .expect("memory worker health");
    assert_eq!(worker.state, WorkerState::Error);
    assert!(
        worker
            .last_error
            .as_ref()
            .is_some_and(|message| message.contains("failed to create memory path")),
        "expected path-aware memory worker error, got {:?}",
        worker.last_error
    );
}

#[test]
fn supervisor_reports_corrupt_memory_store_as_degraded_while_runtime_continues() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join("state");
    let workspace = dir.path().join("workspaces");
    let orch_root = workspace.join("alpha");
    let memory_root = orch_root.join("memory");
    fs::create_dir_all(&memory_root).expect("memory root");
    fs::write(memory_root.join("memory.db"), b"not-a-sqlite-db").expect("corrupt db");

    let settings = serde_yaml::from_str(&settings_yaml(&workspace, true)).expect("settings");
    let stop_path = StatePaths::new(&state_root).stop_signal_path();

    let handle = thread::spawn({
        let state_root = state_root.clone();
        move || run_supervisor(&state_root, settings)
    });

    wait_for_worker_state(
        &state_root,
        "memory_worker",
        WorkerState::Error,
        Duration::from_secs(2),
    );
    let running_state = load_supervisor_state(&StatePaths::new(&state_root)).expect("load state");
    let memory = running_state
        .workers
        .get("memory_worker")
        .expect("memory worker health");
    assert_eq!(memory.state, WorkerState::Error);
    assert!(
        memory
            .last_error
            .as_ref()
            .is_some_and(|message| message.contains("memory_db_corrupt")),
        "expected corruption marker in error, got {:?}",
        memory.last_error
    );
    assert_eq!(
        running_state
            .workers
            .get("orchestrator_dispatcher")
            .map(|worker| worker.state),
        Some(WorkerState::Running),
        "orchestrator dispatcher should continue while memory worker is degraded"
    );

    fs::write(&stop_path, "stop").expect("stop");
    handle.join().expect("join").expect("supervisor");

    let log_file = orch_root.join("logs/orchestrator.log");
    let log_body = fs::read_to_string(log_file).expect("read orchestrator log");
    assert!(
        log_body.lines().any(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .is_some_and(|event| {
                    event["event"] == "memory.worker.degraded"
                        && event["reason_code"] == "memory_db_corrupt"
                })
        }),
        "expected structured degraded event in orchestrator log"
    );
}
