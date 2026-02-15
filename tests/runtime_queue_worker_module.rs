use direclaw::provider::RunnerBinaries;
use direclaw::runtime::queue_worker::drain_queue_once_with_binaries;
use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths};
use tempfile::tempdir;

#[test]
fn runtime_queue_worker_module_exposes_drain_api() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
"#,
    )
    .expect("parse settings");

    let binaries = RunnerBinaries {
        anthropic: "unused".to_string(),
        openai: "unused".to_string(),
    };

    let processed = drain_queue_once_with_binaries(&state_root, &settings, 1, &binaries)
        .expect("drain empty queue");
    assert_eq!(processed, 0);
}
