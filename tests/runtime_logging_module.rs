use direclaw::runtime::logging::append_runtime_log;
use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths};
use std::fs;
use tempfile::tempdir;

#[test]
fn runtime_logging_module_writes_json_log_lines() {
    let tmp = tempdir().expect("tempdir");
    let paths = StatePaths::new(tmp.path().join(".direclaw"));
    bootstrap_state_root(&paths).expect("bootstrap state root");

    append_runtime_log(&paths, "info", "runtime.test", "hello runtime");

    let log = fs::read_to_string(paths.runtime_log_path()).expect("read runtime log");
    assert!(log.contains("\"level\":\"info\""));
    assert!(log.contains("\"event\":\"runtime.test\""));
    assert!(log.contains("\"message\":\"hello runtime\""));
}
