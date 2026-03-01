use direclaw::config::Settings;
use direclaw::queue::{IncomingMessage, OutgoingMessage, QueuePaths};
use direclaw::runtime::heartbeat_worker::{
    build_heartbeat_incoming_message, configured_heartbeat_interval, match_heartbeat_responses,
    resolve_heartbeat_prompt, tick_heartbeat_worker,
};
use direclaw::runtime::queue_worker::drain_queue_once;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tempfile::tempdir;

fn write_orchestrator_config(path: &Path, orchestrator_id: &str, agents: &[&str]) {
    let mut agent_lines = String::new();
    for agent in agents {
        agent_lines.push_str(&format!(
            "  {agent}:\n    provider: openai\n    model: gpt-5.3-codex-spark\n"
        ));
    }

    fs::write(
        path,
        format!(
            r#"
id: {orchestrator_id}
selector_agent: router
default_workflow: triage
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
{agent_lines}workflows:
  - id: triage
    version: 1
    description: heartbeat test workflow
    tags: [heartbeat]
    steps:
      - id: start
        type: agent_task
        agent: router
        prompt: start
        outputs: [summary, artifact]
        output_files:
          summary: outputs/{{{{workflow.step_id}}}}-{{{{workflow.attempt}}}}-summary.txt
          artifact: outputs/{{{{workflow.step_id}}}}-{{{{workflow.attempt}}}}.txt
"#,
        ),
    )
    .expect("write orchestrator");
}

fn write_settings(orchestrators: &[(&str, &Path)]) -> Settings {
    let mut orchestrator_lines = String::new();
    for (orchestrator_id, workspace) in orchestrators {
        orchestrator_lines.push_str(&format!(
            "  {orchestrator_id}:\n    private_workspace: {}\n    shared_access: []\n",
            workspace.display()
        ));
    }

    serde_yaml::from_str(&format!(
        r#"
workspaces_path: /tmp
shared_workspaces: {{}}
orchestrators:
{orchestrator_lines}channel_profiles: {{}}
monitoring:
  heartbeat_interval: 30
channels: {{}}
auth_sync:
  enabled: false
"#
    ))
    .expect("parse settings")
}

fn sample_outgoing(
    message_id: &str,
    agent: &str,
    content: &str,
    conversation_id: Option<&str>,
) -> OutgoingMessage {
    OutgoingMessage {
        channel: "heartbeat".to_string(),
        channel_profile_id: None,
        sender: "heartbeat".to_string(),
        message: content.to_string(),
        original_message: content.to_string(),
        timestamp: 1710000000,
        message_id: message_id.to_string(),
        agent: agent.to_string(),
        conversation_id: conversation_id.map(ToString::to_string),
        target_ref: None,
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    }
}

fn write_heartbeat_prompt(root: &Path, agent: &str, body: &str) {
    let heartbeat_dir = root.join("heartbeat");
    fs::create_dir_all(&heartbeat_dir).expect("heartbeat dir");
    fs::write(heartbeat_dir.join(format!("{agent}.md")), body).expect("heartbeat prompt");
}

#[test]
fn runtime_heartbeat_worker_module_resolves_prompt_from_file_or_none_when_missing() {
    let temp = tempdir().expect("tempdir");
    let orchestrator_root = temp.path().join("orch-a");
    fs::create_dir_all(&orchestrator_root).expect("orchestrator root");

    let missing =
        resolve_heartbeat_prompt(&orchestrator_root, "orch-a", "worker").expect("missing");
    assert_eq!(missing, None);

    write_heartbeat_prompt(&orchestrator_root, "worker", "Custom heartbeat prompt");
    let custom = resolve_heartbeat_prompt(&orchestrator_root, "orch-a", "worker").expect("custom");
    assert_eq!(custom, Some("Custom heartbeat prompt".to_string()));
}

#[test]
fn runtime_heartbeat_worker_module_builds_deterministic_payload_metadata() {
    let message =
        build_heartbeat_incoming_message("orch-a", "worker", "ping", 1700000001).expect("payload");

    assert_eq!(message.channel, "heartbeat");
    assert_eq!(message.sender, "heartbeat:orch-a");
    assert_eq!(message.sender_id, "heartbeat-worker");
    assert_eq!(message.timestamp, 1700000001);
    assert_eq!(
        message.message_id,
        "heartbeat-orch-a-worker-1700000001".to_string()
    );
    assert_eq!(
        message.conversation_id,
        Some("hb:orch-a:worker:1700000001".to_string())
    );
    assert_eq!(
        message.workflow_run_id,
        Some("hb:orch-a:worker:1700000001".to_string())
    );
    assert_eq!(
        message.workflow_step_id,
        Some("heartbeat_worker_check".to_string())
    );
}

#[test]
fn runtime_heartbeat_worker_module_matches_outbound_responses_without_mutation() {
    let temp = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(temp.path());
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    let match_path = queue.outgoing.join("a.json");
    let malformed_path = queue.outgoing.join("b.json");
    fs::write(
        &match_path,
        serde_json::to_vec_pretty(&sample_outgoing(
            "heartbeat-orch-a-worker-10",
            "worker",
            "healthy",
            Some("hb:orch-a:worker:10"),
        ))
        .expect("serialize outgoing"),
    )
    .expect("write match");
    fs::write(&malformed_path, b"{not-json").expect("write malformed");

    let matched = match_heartbeat_responses(
        &queue,
        "orch-a",
        "worker",
        "heartbeat-orch-a-worker-10",
        "hb:orch-a:worker:10",
    )
    .expect("match");
    assert_eq!(matched, Some("healthy".to_string()));

    let missing = match_heartbeat_responses(
        &queue,
        "orch-a",
        "worker",
        "missing",
        "hb:orch-a:different-worker:10",
    )
    .expect("missing");
    assert!(missing.is_none());

    assert!(match_path.exists(), "matched outbound file must remain");
    assert!(
        malformed_path.exists(),
        "malformed outbound file must remain"
    );
}

#[test]
fn runtime_heartbeat_worker_module_ignores_stale_response_from_prior_tick() {
    let temp = tempdir().expect("tempdir");
    let queue = QueuePaths::from_state_root(temp.path());
    fs::create_dir_all(&queue.outgoing).expect("outgoing");

    fs::write(
        queue.outgoing.join("stale.json"),
        serde_json::to_vec_pretty(&sample_outgoing(
            "heartbeat-orch-a-worker-10",
            "worker",
            "healthy",
            Some("hb:orch-a:worker:10"),
        ))
        .expect("serialize outgoing"),
    )
    .expect("write stale");

    let matched = match_heartbeat_responses(
        &queue,
        "orch-a",
        "worker",
        "heartbeat-orch-a-worker-11",
        "hb:orch-a:worker:11",
    )
    .expect("match");
    assert!(
        matched.is_none(),
        "stale response should not satisfy a newer heartbeat tick"
    );
}

#[test]
fn runtime_heartbeat_worker_module_tick_enqueues_all_agents_across_orchestrators() {
    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");

    let orch_a = temp.path().join("orch-a");
    let orch_b = temp.path().join("orch-b");
    write_heartbeat_prompt(&orch_a, "worker-a", "heartbeat from worker-a");
    write_heartbeat_prompt(&orch_b, "worker-b", "heartbeat from file");

    write_orchestrator_config(&orch_a.join("orchestrator.yaml"), "orch_a", &["worker-a"]);
    write_orchestrator_config(
        &orch_b.join("orchestrator.yaml"),
        "orch_b",
        &["worker-b", "worker-c"],
    );

    let settings = write_settings(&[("orch_a", &orch_a), ("orch_b", &orch_b)]);

    tick_heartbeat_worker(&state_root, &settings).expect("heartbeat tick");

    let queue_a = QueuePaths::from_state_root(&orch_a);
    let queue_b = QueuePaths::from_state_root(&orch_b);

    let entries_a = fs::read_dir(&queue_a.incoming)
        .expect("incoming a")
        .filter_map(|entry| entry.ok())
        .count();
    let entries_b = fs::read_dir(&queue_b.incoming)
        .expect("incoming b")
        .filter_map(|entry| entry.ok())
        .count();

    assert_eq!(entries_a, 1, "only worker-a has heartbeat prompt");
    assert_eq!(entries_b, 1, "only worker-b has heartbeat prompt");

    let worker_b_payload = fs::read_dir(&queue_b.incoming)
        .expect("incoming b")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find_map(|path| {
            let raw = fs::read_to_string(path).ok()?;
            let payload: IncomingMessage = serde_json::from_str(&raw).ok()?;
            (payload.sender_id == "heartbeat-worker-b").then_some(payload)
        })
        .expect("worker-b payload");
    assert_eq!(worker_b_payload.message, "heartbeat from file");

    let worker_c_payload = fs::read_dir(&queue_b.incoming)
        .expect("incoming b")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find_map(|path| {
            let raw = fs::read_to_string(path).ok()?;
            let payload: IncomingMessage = serde_json::from_str(&raw).ok()?;
            (payload.sender_id == "heartbeat-worker-c").then_some(payload)
        });
    assert!(
        worker_c_payload.is_none(),
        "missing heartbeat.md should skip enqueue"
    );

    let runtime_log = fs::read_to_string(state_root.join("logs/runtime.log")).expect("log");
    assert!(runtime_log.contains("heartbeat.prompt.missing"));
    assert!(runtime_log.contains("orchestrator=orch_b"));
    assert!(runtime_log.contains("agent=worker-c"));
}

#[test]
fn runtime_heartbeat_worker_module_exposes_configured_interval() {
    let disabled: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring:
  heartbeat_interval: 0
channels: {}
"#,
    )
    .expect("parse settings");
    assert_eq!(configured_heartbeat_interval(&disabled), None);

    let enabled: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring:
  heartbeat_interval: 30
channels: {}
"#,
    )
    .expect("parse settings");
    assert_eq!(
        configured_heartbeat_interval(&enabled),
        Some(Duration::from_secs(30))
    );

    let implicit_default: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
"#,
    )
    .expect("parse settings");
    assert_eq!(
        configured_heartbeat_interval(&implicit_default),
        Some(Duration::from_secs(3600))
    );
}

#[test]
fn runtime_heartbeat_worker_module_logs_matched_response_metadata() {
    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");

    let orch = temp.path().join("orch");
    write_heartbeat_prompt(&orch, "worker", "heartbeat from worker");
    write_orchestrator_config(&orch.join("orchestrator.yaml"), "orch", &["worker"]);
    let settings = write_settings(&[("orch", &orch)]);

    let queue = QueuePaths::from_state_root(&orch);
    fs::create_dir_all(&queue.outgoing).expect("outgoing");
    let matched = sample_outgoing(
        "heartbeat-orch-worker-1700000001",
        "worker",
        "agent looks healthy",
        Some("hb:orch:worker:1700000001"),
    );
    fs::write(
        queue.outgoing.join("heartbeat-orch-worker-1700000001.json"),
        serde_json::to_vec_pretty(&matched).expect("serialize"),
    )
    .expect("write outgoing");

    std::env::set_var("DIRECLAW_HEARTBEAT_TICK_AT", "1700000001");
    tick_heartbeat_worker(&state_root, &settings).expect("tick");
    std::env::remove_var("DIRECLAW_HEARTBEAT_TICK_AT");

    let runtime_log = fs::read_to_string(state_root.join("logs/runtime.log")).expect("log");
    assert!(runtime_log.contains("heartbeat.response.matched"));
    assert!(runtime_log.contains("orchestrator=orch"));
    assert!(runtime_log.contains("agent=worker"));
    assert!(runtime_log.contains("message_id=heartbeat-orch-worker-1700000001"));
}

#[test]
fn runtime_heartbeat_worker_module_routes_heartbeat_messages_through_queue_lifecycle() {
    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");

    let orch = temp.path().join("orch");
    write_heartbeat_prompt(&orch, "router", "router heartbeat");
    write_heartbeat_prompt(&orch, "worker", "worker heartbeat");
    write_orchestrator_config(&orch.join("orchestrator.yaml"), "orch", &["worker"]);
    let settings = write_settings(&[("orch", &orch)]);

    tick_heartbeat_worker(&state_root, &settings).expect("tick");
    let processed = drain_queue_once(&state_root, &settings, 1).expect("drain queue");
    assert_eq!(
        processed, 2,
        "router + worker heartbeat messages must process"
    );

    let queue = QueuePaths::from_state_root(&orch);
    let outgoing_count = fs::read_dir(&queue.outgoing)
        .expect("outgoing")
        .filter_map(|entry| entry.ok())
        .count();
    assert!(
        outgoing_count >= 2,
        "expected heartbeat responses in outgoing queue"
    );
}

#[test]
fn runtime_heartbeat_worker_module_does_not_match_responses_across_ticks_without_fixed_timestamp() {
    let temp = tempdir().expect("tempdir");
    let state_root = temp.path().join(".direclaw");

    let orch = temp.path().join("orch");
    write_heartbeat_prompt(&orch, "worker", "worker heartbeat");
    write_orchestrator_config(&orch.join("orchestrator.yaml"), "orch", &["worker"]);
    let settings = write_settings(&[("orch", &orch)]);

    std::env::set_var("DIRECLAW_HEARTBEAT_TICK_AT", "1700000001");
    tick_heartbeat_worker(&state_root, &settings).expect("first tick");
    std::env::remove_var("DIRECLAW_HEARTBEAT_TICK_AT");
    let _processed = drain_queue_once(&state_root, &settings, 1).expect("drain queue");
    tick_heartbeat_worker(&state_root, &settings).expect("second tick");

    let runtime_log = fs::read_to_string(state_root.join("logs/runtime.log")).expect("log");
    assert!(
        runtime_log.contains("heartbeat.response.missing"),
        "expected stale heartbeat responses to be ignored across ticks"
    );
}
