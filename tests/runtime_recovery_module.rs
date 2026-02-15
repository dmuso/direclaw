use direclaw::queue::QueuePaths;
use direclaw::runtime::recovery::recover_processing_queue_entries;
use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths};
use std::fs;
use tempfile::tempdir;

#[test]
fn runtime_recovery_module_requeues_processing_entries_to_incoming() {
    let tmp = tempdir().expect("tempdir");
    let state_root = tmp.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let queue = QueuePaths::from_state_root(&state_root);
    fs::write(queue.processing.join("task.json"), b"{}").expect("write processing file");

    let recovered = recover_processing_queue_entries(&state_root).expect("recover entries");

    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].starts_with(&queue.incoming));
    assert!(queue
        .processing
        .read_dir()
        .expect("read processing")
        .next()
        .is_none());
}
