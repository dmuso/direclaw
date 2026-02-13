use direclaw::runtime::{bootstrap_state_root, StatePaths};
use tempfile::tempdir;

#[test]
fn bootstrapping_empty_state_root_creates_full_tree() {
    let tmp = tempdir().expect("tempdir");
    let state_root = tmp.path().join(".direclaw");
    let paths = StatePaths::new(&state_root);

    bootstrap_state_root(&paths).expect("bootstrap state root");

    for dir in paths.required_directories() {
        assert!(dir.is_dir(), "expected directory {}", dir.display());
    }
}
