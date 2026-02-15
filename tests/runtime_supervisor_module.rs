use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths};
use direclaw::runtime::supervisor::{
    cleanup_stale_supervisor, clear_start_lock, load_supervisor_state, reserve_start_lock,
    save_supervisor_state, supervisor_ownership_state, OwnershipState, SupervisorState,
};
use std::collections::BTreeMap;
use std::fs;
use tempfile::tempdir;

#[test]
fn runtime_supervisor_module_exposes_supervisor_state_and_lock_apis() {
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

    assert_eq!(
        supervisor_ownership_state(&paths).expect("ownership"),
        OwnershipState::Stale
    );

    cleanup_stale_supervisor(&paths).expect("cleanup");
    let cleaned = load_supervisor_state(&paths).expect("load");

    assert!(!cleaned.running);
    assert!(cleaned.pid.is_none());

    reserve_start_lock(&paths).expect("reserve");
    clear_start_lock(&paths);
}
