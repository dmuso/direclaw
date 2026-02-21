use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths};
use direclaw::runtime::supervisor::{
    cleanup_stale_supervisor, clear_start_lock, load_supervisor_state, reserve_start_lock,
    run_supervisor, save_supervisor_state, signal_stop, supervisor_ownership_state, OwnershipState,
    SupervisorState,
};
use direclaw::{config::Settings, runtime::RuntimeError};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;
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
        slack_profiles: Vec::new(),
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

#[test]
fn runtime_supervisor_module_exposes_supervisor_loop_entrypoint() {
    let _: fn(&Path, Settings) -> Result<(), RuntimeError> = run_supervisor;
}

#[test]
fn runtime_supervisor_logs_prompt_validation_warnings_on_startup() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    let paths = StatePaths::new(&state_root);
    bootstrap_state_root(&paths).expect("bootstrap");

    let orchestrator_workspace = dir.path().join("orch");
    fs::create_dir_all(&orchestrator_workspace).expect("orchestrator workspace");
    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: router
default_workflow: triage
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.2
workflows:
  - id: triage
    version: 1
    description: "triage"
    tags: [triage]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: "inline prompt"
        outputs: [summary, artifact]
        output_files:
          summary: outputs/summary.txt
          artifact: outputs/artifact.txt
"#,
    )
    .expect("write orchestrator");
    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles: {{}}
monitoring:
  heartbeat_interval: 0
channels: {{}}
"#,
        workspace = dir.path().display(),
        orchestrator_workspace = orchestrator_workspace.display()
    ))
    .expect("settings");

    let root_for_thread = state_root.clone();
    let settings_for_thread = settings.clone();
    let handle = thread::spawn(move || run_supervisor(&root_for_thread, settings_for_thread));
    thread::sleep(Duration::from_millis(250));
    signal_stop(&paths).expect("signal stop");
    handle.join().expect("join").expect("supervisor run");

    let runtime_log =
        fs::read_to_string(state_root.join("logs/runtime.log")).expect("read runtime log");
    assert!(
        runtime_log.contains("\"event\":\"prompts.validation.issue\""),
        "expected prompt validation warning in runtime log:\n{runtime_log}"
    );
    assert!(
        runtime_log.contains("prompt is inline; expected relative markdown path"),
        "expected informative prompt warning in runtime log:\n{runtime_log}"
    );
}
