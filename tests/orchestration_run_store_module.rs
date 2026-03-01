use direclaw::orchestration::run_store::{RunState, WorkflowRunStore};
use tempfile::tempdir;

#[test]
fn run_store_module_persists_runs_and_progress() {
    let temp = tempdir().expect("tempdir");
    let store = WorkflowRunStore::new(temp.path());

    let run = store
        .create_run("run-1", "wf-default", 42)
        .expect("create run");

    assert_eq!(run.state, RunState::Queued);

    let loaded = store.load_run("run-1").expect("load run");
    assert_eq!(loaded.workflow_id, "wf-default");

    let progress = store.load_progress("run-1").expect("load progress");
    assert_eq!(progress.summary, "queued");
    assert_eq!(progress.next_expected_action, "workflow start");
}

#[test]
fn run_store_module_loads_latest_run_by_source_message_id() {
    let temp = tempdir().expect("tempdir");
    let store = WorkflowRunStore::new(temp.path());

    let mut run_1 = store
        .create_run("run-1", "wf-default", 10)
        .expect("create run 1");
    run_1.source_message_id = Some("msg-1".to_string());
    run_1.updated_at = 11;
    store.persist_run(&run_1).expect("persist run 1");

    let mut run_2 = store
        .create_run("run-2", "wf-default", 20)
        .expect("create run 2");
    run_2.source_message_id = Some("msg-1".to_string());
    run_2.updated_at = 21;
    store.persist_run(&run_2).expect("persist run 2");

    let latest = store
        .latest_run_for_source_message_id("msg-1")
        .expect("lookup latest")
        .expect("run should exist");
    assert_eq!(latest.run_id, "run-2");

    let missing = store
        .latest_run_for_source_message_id("msg-unknown")
        .expect("lookup missing");
    assert!(missing.is_none());
}
