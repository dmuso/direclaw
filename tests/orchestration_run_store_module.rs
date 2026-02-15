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
