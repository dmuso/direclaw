use direclaw::queue::logging::append_queue_log;
use direclaw::queue::QueuePaths;
use std::fs;
use tempfile::tempdir;

#[test]
fn queue_logging_module_appends_security_log_entries() {
    let temp = tempdir().expect("tempdir");
    let paths = QueuePaths::from_state_root(temp.path());

    append_queue_log(&paths, "first entry");
    append_queue_log(&paths, "second entry");

    let log_path = temp.path().join("logs/security.log");
    let logged = fs::read_to_string(log_path).expect("read security log");
    assert_eq!(logged, "first entry\nsecond entry\n");
}
