use direclaw::runtime::{load_supervisor_state, run_supervisor, StatePaths, WorkerState};
use std::fs;
use std::thread;
use std::time::Duration;
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

#[test]
fn supervisor_registers_memory_worker_only_when_memory_enabled() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join("state");
    let workspace = dir.path().join("workspaces");
    fs::create_dir_all(workspace.join("alpha")).expect("workspace");

    for enabled in [true, false] {
        let settings = serde_yaml::from_str(&settings_yaml(&workspace, enabled)).expect("settings");
        let root = state_root.join(if enabled { "enabled" } else { "disabled" });
        fs::create_dir_all(&root).expect("state root");
        let stop_path = StatePaths::new(&root).stop_signal_path();

        let handle = thread::spawn({
            let root = root.clone();
            move || run_supervisor(&root, settings)
        });

        thread::sleep(Duration::from_millis(350));
        fs::write(&stop_path, "stop").expect("stop");
        handle.join().expect("join").expect("supervisor");

        let state = load_supervisor_state(&StatePaths::new(&root)).expect("load state");
        let has_memory_worker = state.workers.contains_key("memory_worker");
        assert_eq!(has_memory_worker, enabled);

        if enabled {
            let runtime_root = workspace.join("alpha");
            let memory_root = runtime_root.join("memory");
            let ingest = memory_root.join("ingest");
            let processed = ingest.join("processed");
            let rejected = ingest.join("rejected");
            let bulletins = memory_root.join("bulletins");
            let log_file = memory_root.join("logs/memory.log");

            assert!(ingest.is_dir(), "missing {}", ingest.display());
            assert!(processed.is_dir(), "missing {}", processed.display());
            assert!(rejected.is_dir(), "missing {}", rejected.display());
            assert!(bulletins.is_dir(), "missing {}", bulletins.display());
            assert!(log_file.is_file(), "missing {}", log_file.display());

            let log_body = fs::read_to_string(&log_file).expect("read memory log");
            assert!(
                log_body.contains("memory worker bootstrap complete"),
                "expected bootstrap line in memory log"
            );
        }
    }
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

    thread::sleep(Duration::from_millis(450));
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
