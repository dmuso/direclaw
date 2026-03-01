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

#[test]
fn run_store_module_resolves_latest_run_by_conversation() {
    let temp = tempdir().expect("tempdir");
    let store = WorkflowRunStore::new(temp.path());

    let mut old_run = store
        .create_run("run-old", "wf-default", 10)
        .expect("create old run");
    old_run.channel_profile_id = Some("engineering".to_string());
    old_run.status_conversation_id = Some("C1:100.1".to_string());
    old_run.state = RunState::Failed;
    old_run.updated_at = 20;
    store.persist_run(&old_run).expect("persist old run");

    let mut new_run = store
        .create_run("run-new", "wf-default", 30)
        .expect("create new run");
    new_run.channel_profile_id = Some("engineering".to_string());
    new_run.status_conversation_id = Some("C1:100.1".to_string());
    new_run.state = RunState::Running;
    new_run.updated_at = 40;
    store.persist_run(&new_run).expect("persist new run");

    let mut terminal_run = store
        .create_run("run-terminal", "wf-default", 50)
        .expect("create terminal run");
    terminal_run.channel_profile_id = Some("engineering".to_string());
    terminal_run.status_conversation_id = Some("C1:100.1".to_string());
    terminal_run.state = RunState::Failed;
    terminal_run.updated_at = 60;
    store
        .persist_run(&terminal_run)
        .expect("persist terminal run");

    let active = store
        .latest_run_for_conversation("engineering", "C1:100.1", false)
        .expect("active lookup")
        .expect("active run");
    assert_eq!(active.run_id, "run-new");

    let terminal_only = store
        .latest_run_for_conversation("engineering", "C1:100.1", true)
        .expect("terminal lookup")
        .expect("terminal run");
    assert_eq!(terminal_only.run_id, "run-terminal");
}
