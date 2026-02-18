use direclaw::config::Settings;
use direclaw::orchestration::routing::{
    process_queued_message_with_runner_binaries, FunctionRegistry,
};
use direclaw::orchestration::run_store::WorkflowRunStore;
use direclaw::orchestration::scheduler::{ScheduledTriggerEnvelope, TargetAction};
use direclaw::orchestration::transitions::RoutedSelectorAction;
use direclaw::provider::RunnerBinaries;
use direclaw::queue::IncomingMessage;
use serde_json::Map;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

fn write_script(path: &std::path::Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn write_orchestrator(path: &std::path::Path) {
    fs::create_dir_all(path).expect("create orchestrator dir");
    fs::write(
        path.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: router
default_workflow: triage
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  router:
    provider: openai
    model: gpt-5.2
    can_orchestrate_workflows: true
workflows:
  - id: triage
    version: 1
    description: default
    tags: [triage]
    steps:
      - id: start
        type: agent_task
        agent: router
        prompt: say ok
        outputs: [summary, artifact]
        output_files:
          summary: out/summary.txt
          artifact: out/artifact.txt
"#,
    )
    .expect("write orchestrator yaml");
}

fn settings_for(
    private_workspace: &std::path::Path,
    workspaces_root: &std::path::Path,
) -> Settings {
    serde_yaml::from_str(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
        workspaces_root.display(),
        private_workspace.display()
    ))
    .expect("settings")
}

#[test]
fn scheduled_workflow_start_routes_through_existing_action_path() {
    let temp = tempdir().expect("tempdir");
    let workspaces_root = temp.path().join("workspaces");
    let private_workspace = workspaces_root.join("main");
    write_orchestrator(&private_workspace);

    let codex = temp.path().join("codex-mock");
    write_script(
        &codex,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );

    let settings = settings_for(&private_workspace, temp.path());
    let runtime_root = settings
        .resolve_orchestrator_runtime_root("main")
        .expect("runtime root");
    let functions = FunctionRegistry::v1_defaults(WorkflowRunStore::new(&runtime_root), &settings);

    let scheduled_inputs = Map::from_iter([(
        "incidentId".to_string(),
        serde_json::Value::String("INC-42".to_string()),
    )]);
    let envelope = ScheduledTriggerEnvelope {
        job_id: "job-1".to_string(),
        execution_id: "exec-1".to_string(),
        triggered_at: 1_700_000_000,
        orchestrator_id: "main".to_string(),
        target_action: TargetAction::WorkflowStart {
            workflow_id: "triage".to_string(),
            inputs: scheduled_inputs.clone(),
        },
        target_ref: None,
    };
    let inbound = IncomingMessage {
        channel: "scheduler".to_string(),
        channel_profile_id: None,
        sender: "scheduler:main".to_string(),
        sender_id: "job-1".to_string(),
        message: serde_json::to_string(&envelope).expect("serialize envelope"),
        timestamp: 1_700_000_000,
        message_id: "exec-1".to_string(),
        conversation_id: Some("scheduler:job-1".to_string()),
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let action = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1_700_000_001,
        &BTreeMap::new(),
        &functions,
        Some(RunnerBinaries {
            anthropic: codex.display().to_string(),
            openai: codex.display().to_string(),
        }),
        |_attempt, _request, _orchestrator| None,
    )
    .expect("scheduled routing");

    match action {
        RoutedSelectorAction::WorkflowStart {
            workflow_id,
            run_id,
        } => {
            assert_eq!(workflow_id, "triage");
            let run = WorkflowRunStore::new(&runtime_root)
                .load_run(&run_id)
                .expect("load run");
            assert_eq!(
                run.inputs.get("workflow_inputs"),
                Some(&serde_json::Value::Object(scheduled_inputs))
            );
        }
        other => panic!("expected workflow start action, got {other:?}"),
    }
}

#[test]
fn scheduled_command_invoke_routes_through_existing_validation_path() {
    let temp = tempdir().expect("tempdir");
    let workspaces_root = temp.path().join("workspaces");
    let private_workspace = workspaces_root.join("main");
    write_orchestrator(&private_workspace);
    let settings = settings_for(&private_workspace, temp.path());

    let runtime_root = settings
        .resolve_orchestrator_runtime_root("main")
        .expect("runtime root");
    let functions = FunctionRegistry::v1_defaults(WorkflowRunStore::new(&runtime_root), &settings);

    let envelope = ScheduledTriggerEnvelope {
        job_id: "job-2".to_string(),
        execution_id: "exec-2".to_string(),
        triggered_at: 1_700_000_000,
        orchestrator_id: "main".to_string(),
        target_action: TargetAction::CommandInvoke {
            function_id: "orchestrator.list".to_string(),
            function_args: Map::new(),
        },
        target_ref: None,
    };
    let inbound = IncomingMessage {
        channel: "scheduler".to_string(),
        channel_profile_id: None,
        sender: "scheduler:main".to_string(),
        sender_id: "job-2".to_string(),
        message: serde_json::to_string(&envelope).expect("serialize envelope"),
        timestamp: 1_700_000_000,
        message_id: "exec-2".to_string(),
        conversation_id: Some("scheduler:job-2".to_string()),
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let action = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1_700_000_001,
        &BTreeMap::new(),
        &functions,
        None,
        |_attempt, _request, _orchestrator| None,
    )
    .expect("scheduled routing");

    match action {
        RoutedSelectorAction::CommandInvoke { result } => {
            let orchestrators = result
                .get("orchestrators")
                .and_then(|value| value.as_array())
                .expect("orchestrators array");
            assert!(orchestrators
                .iter()
                .any(|value| value.as_str() == Some("main")));
        }
        other => panic!("expected command invoke action, got {other:?}"),
    }
}

#[test]
fn scheduled_trigger_failure_records_scheduler_failure_event() {
    let temp = tempdir().expect("tempdir");
    let workspaces_root = temp.path().join("workspaces");
    let private_workspace = workspaces_root.join("main");
    write_orchestrator(&private_workspace);
    let settings = settings_for(&private_workspace, temp.path());

    let runtime_root = settings
        .resolve_orchestrator_runtime_root("main")
        .expect("runtime root");
    let functions = FunctionRegistry::v1_defaults(WorkflowRunStore::new(&runtime_root), &settings);

    let envelope = ScheduledTriggerEnvelope {
        job_id: "job-fail".to_string(),
        execution_id: "exec-fail".to_string(),
        triggered_at: 1_700_000_000,
        orchestrator_id: "main".to_string(),
        target_action: TargetAction::WorkflowStart {
            workflow_id: "missing-workflow".to_string(),
            inputs: Map::new(),
        },
        target_ref: None,
    };
    let inbound = IncomingMessage {
        channel: "scheduler".to_string(),
        channel_profile_id: None,
        sender: "scheduler:main".to_string(),
        sender_id: "job-fail".to_string(),
        message: serde_json::to_string(&envelope).expect("serialize envelope"),
        timestamp: 1_700_000_000,
        message_id: "exec-fail".to_string(),
        conversation_id: Some("scheduler:job-fail".to_string()),
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1_700_000_001,
        &BTreeMap::new(),
        &functions,
        None,
        |_attempt, _request, _orchestrator| None,
    )
    .expect_err("scheduled routing should fail for unknown workflow");

    let log_path = runtime_root.join("logs/orchestrator.log");
    let log = fs::read_to_string(&log_path).expect("read log");
    assert!(
        log.contains("\"event\":\"scheduler.trigger.failed\""),
        "missing failure event in log: {log}"
    );
}
