use direclaw::orchestration::scheduler::{ScheduledTriggerEnvelope, TargetAction};
use direclaw::provider::RunnerBinaries;
use direclaw::queue::{IncomingMessage, OutgoingMessage, QueuePaths};
use direclaw::runtime::queue_worker::drain_queue_once_with_binaries;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

fn write_script(path: &std::path::Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

#[test]
fn scheduled_slack_target_is_propagated_to_outbound_queue_payload() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let orch_ws = workspace_root.join("main");
    fs::create_dir_all(&orch_ws).expect("orchestrator workspace");
    fs::write(
        orch_ws.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: router
default_workflow: triage
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  router:
    provider: openai
    model: gpt-5.3-codex-spark
    can_orchestrate_workflows: true
workflows:
  - id: triage
    version: 1
    description: default
    tags: [triage]
    steps:
      - id: run
        type: agent_task
        agent: router
        prompt: run
        outputs: [summary, artifact]
        output_files:
          summary: out/summary.txt
          artifact: out/artifact.txt
"#,
    )
    .expect("write orchestrator");

    let settings: direclaw::config::Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  slack_main:
    channel: slack
    orchestrator_id: main
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        orch_ws.display()
    ))
    .expect("settings");

    let runtime_root = settings
        .resolve_orchestrator_runtime_root("main")
        .expect("runtime root");
    let queue = QueuePaths::from_state_root(&runtime_root);
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let envelope = ScheduledTriggerEnvelope {
        job_id: "job-1".to_string(),
        execution_id: "exec-1".to_string(),
        triggered_at: 1_700_000_000,
        orchestrator_id: "main".to_string(),
        target_action: TargetAction::WorkflowStart {
            workflow_id: "triage".to_string(),
            inputs: serde_json::Map::new(),
        },
        target_ref: Some(serde_json::json!({
            "channel": "slack",
            "channelProfileId": "slack_main",
            "channelId": "C200",
            "threadTs": "1700000000.1",
            "postingMode": "thread_reply"
        })),
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
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };
    fs::write(
        queue.incoming.join("exec-1.json"),
        serde_json::to_vec_pretty(&inbound).expect("encode inbound"),
    )
    .expect("write inbound");

    let codex = temp.path().join("codex-mock");
    write_script(
        &codex,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"done\\\"}[/workflow_result]\"}}'\n",
    );
    let processed = drain_queue_once_with_binaries(
        temp.path(),
        &settings,
        1,
        &RunnerBinaries {
            anthropic: codex.display().to_string(),
            openai: codex.display().to_string(),
        },
    )
    .expect("drain");
    assert_eq!(processed, 1);

    let mut outgoing_files: Vec<_> = fs::read_dir(&queue.outgoing)
        .expect("outgoing list")
        .map(|entry| entry.expect("entry").path())
        .collect();
    outgoing_files.sort();
    let outgoing: OutgoingMessage = serde_json::from_str(
        &fs::read_to_string(outgoing_files.last().expect("outgoing file")).expect("outgoing"),
    )
    .expect("parse outgoing");

    assert_eq!(outgoing.channel, "slack");
    assert_eq!(outgoing.channel_profile_id.as_deref(), Some("slack_main"));
    assert_eq!(
        outgoing.conversation_id.as_deref(),
        Some("C200:1700000000.1")
    );
    let target_ref = outgoing
        .target_ref
        .expect("expected slack target_ref in outbound payload");
    assert_eq!(target_ref["channelProfileId"], "slack_main");
    assert_eq!(target_ref["channelId"], "C200");
}
