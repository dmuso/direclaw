use direclaw::runtime::ownership_lock::{
    cleanup_stale_supervisor, clear_start_lock, reserve_start_lock, supervisor_ownership_state,
    OwnershipState,
};
use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths};
use std::fs;
use tempfile::tempdir;

#[test]
fn runtime_ownership_lock_module_exposes_lock_and_ownership_apis() {
    let dir = tempdir().expect("tempdir");
    let paths = StatePaths::new(dir.path().join(".direclaw"));
    bootstrap_state_root(&paths).expect("bootstrap");

    fs::write(paths.supervisor_lock_path(), "999999").expect("lock");
    assert_eq!(
        supervisor_ownership_state(&paths).expect("ownership"),
        OwnershipState::Stale
    );

    cleanup_stale_supervisor(&paths).expect("cleanup stale");
    assert_eq!(
        supervisor_ownership_state(&paths).expect("ownership after cleanup"),
        OwnershipState::NotRunning
    );

    reserve_start_lock(&paths).expect("reserve");
    clear_start_lock(&paths);
}
