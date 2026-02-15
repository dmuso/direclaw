use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths, DEFAULT_STATE_ROOT_DIR};
use tempfile::tempdir;

#[test]
fn runtime_state_paths_module_exposes_state_root_bootstrap_apis() {
    assert_eq!(DEFAULT_STATE_ROOT_DIR, ".direclaw");

    let tmp = tempdir().expect("tempdir");
    let state_root = tmp.path().join(".direclaw");
    let paths = StatePaths::new(&state_root);

    bootstrap_state_root(&paths).expect("bootstrap state root");

    for dir in paths.required_directories() {
        assert!(dir.is_dir(), "expected directory {}", dir.display());
    }
}
