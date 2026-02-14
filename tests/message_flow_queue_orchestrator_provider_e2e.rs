use direclaw::config::Settings;
use direclaw::provider::RunnerBinaries;
use direclaw::queue::{IncomingMessage, QueuePaths};
use direclaw::runtime::{
    bootstrap_state_root, drain_queue_once_with_binaries, recover_processing_queue_entries,
    run_supervisor, signal_stop, StatePaths,
};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn write_script(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn sample_message(message_id: &str, conversation_id: &str) -> IncomingMessage {
    IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("eng".to_string()),
        sender: "Dana".to_string(),
        sender_id: "U42".to_string(),
        message: "help".to_string(),
        timestamp: 100,
        message_id: message_id.to_string(),
        conversation_id: Some(conversation_id.to_string()),
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    }
}

fn write_incoming(queue: &QueuePaths, payload: &IncomingMessage) {
    fs::write(
        queue.incoming.join(format!("{}.json", payload.message_id)),
        serde_json::to_vec(payload).expect("serialize"),
    )
    .expect("write incoming");
}

fn write_settings_and_orchestrator(
    temp_root: &Path,
    orchestrator_workspace: &Path,
    selector_provider: &str,
    selection_max_retries: u32,
) -> Settings {
    fs::create_dir_all(orchestrator_workspace).expect("orchestrator workspace");
    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
        format!(
            r#"
id: eng_orchestrator
selector_agent: router
default_workflow: triage
selection_max_retries: {selection_max_retries}
agents:
  router:
    provider: {selector_provider}
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.2
workflows:
  - id: triage
    version: 1
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
"#
        ),
    )
    .expect("write orchestrator");

    serde_yaml::from_str(&format!(
        r#"
workspace_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  eng_orchestrator:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles:
  eng:
    channel: slack
    orchestrator_id: eng_orchestrator
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspace = temp_root.display(),
        orchestrator_workspace = orchestrator_workspace.display()
    ))
    .expect("settings")
}

fn binaries(anthropic: impl Into<String>, openai: impl Into<String>) -> RunnerBinaries {
    RunnerBinaries {
        anthropic: anthropic.into(),
        openai: openai.into(),
    }
}

fn read_outgoing_text(state_root: &Path) -> String {
    let out_dir = state_root.join("queue/outgoing");
    let mut files: Vec<PathBuf> = fs::read_dir(&out_dir)
        .expect("read outgoing")
        .map(|e| e.expect("entry").path())
        .collect();
    files.sort();
    let path = files.pop().expect("outgoing file");
    fs::read_to_string(path).expect("outgoing text")
}

#[test]
fn queue_to_orchestrator_runtime_path_runs_provider_and_persists_selector_artifacts() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let queue = QueuePaths::from_state_root(&state_root);

    let claude = dir.path().join("claude-mock");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-1\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );

    let settings =
        write_settings_and_orchestrator(dir.path(), &dir.path().join("orch"), "anthropic", 1);
    write_incoming(&queue, &sample_message("msg-1", "thread-1"));

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        4,
        &binaries(claude.display().to_string(), "unused"),
    )
    .expect("drain");
    assert_eq!(processed, 1);

    let outgoing = read_outgoing_text(&state_root);
    assert!(outgoing.contains("workflow started"));
    assert!(state_root
        .join("orchestrator/messages/msg-1.json")
        .is_file());
    assert!(state_root
        .join("orchestrator/select/results/sel-msg-1.json")
        .is_file());
    assert!(state_root
        .join("orchestrator/select/logs/sel-msg-1_attempt_0.invocation.json")
        .is_file());
}

#[test]
fn queue_failures_requeue_without_payload_loss_for_unknown_profile() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let queue = QueuePaths::from_state_root(&state_root);

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspace_path: {workspace}
shared_workspaces: {{}}
orchestrators: {{}}
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display()
    ))
    .expect("settings");

    let raw = serde_json::to_string(&sample_message("msg-unknown", "thread-1")).expect("raw");
    fs::write(queue.incoming.join("msg-unknown.json"), &raw).expect("incoming");

    let err =
        drain_queue_once_with_binaries(&state_root, &settings, 2, &binaries("unused", "unused"))
            .expect_err("must fail");
    assert!(err.contains("unknown channel profile"));

    let mut incoming_files: Vec<_> = fs::read_dir(&queue.incoming)
        .expect("incoming")
        .map(|e| e.expect("entry").path())
        .collect();
    incoming_files.sort();
    assert_eq!(incoming_files.len(), 1);
    assert_eq!(
        fs::read_to_string(&incoming_files[0]).expect("requeued"),
        raw
    );
    assert!(fs::read_dir(&queue.processing)
        .expect("processing")
        .next()
        .is_none());
}

#[test]
fn provider_non_zero_and_parse_failures_are_logged_and_fall_back_deterministically() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let queue = QueuePaths::from_state_root(&state_root);

    let claude_fail = dir.path().join("claude-fail");
    write_script(&claude_fail, "#!/bin/sh\necho fail 1>&2\nexit 7\n");
    let settings_fail =
        write_settings_and_orchestrator(dir.path(), &dir.path().join("orch-fail"), "anthropic", 1);
    write_incoming(&queue, &sample_message("msg-fail", "thread-1"));
    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings_fail,
        2,
        &binaries(claude_fail.display().to_string(), "unused"),
    )
    .expect("drain non-zero");
    assert_eq!(processed, 1);
    let non_zero_log = fs::read_to_string(
        state_root.join("orchestrator/select/logs/sel-msg-fail_attempt_0.invocation.json"),
    )
    .expect("non-zero log");
    assert!(non_zero_log.contains("\"status\": \"failed\""));
    assert!(non_zero_log.contains("\"exitCode\": 7"));

    let codex_bad = dir.path().join("codex-bad");
    write_script(&codex_bad, "#!/bin/sh\necho '{not-json}'\n");
    let settings_parse =
        write_settings_and_orchestrator(dir.path(), &dir.path().join("orch-parse"), "openai", 1);
    write_incoming(&queue, &sample_message("msg-parse", "thread-2"));
    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings_parse,
        2,
        &binaries("unused", codex_bad.display().to_string()),
    )
    .expect("drain parse failure");
    assert_eq!(processed, 1);
    let parse_log = fs::read_to_string(
        state_root.join("orchestrator/select/logs/sel-msg-parse_attempt_0.invocation.json"),
    )
    .expect("parse log");
    assert!(parse_log.contains("\"status\": \"failed\""));
    assert!(
        parse_log.contains("invalid jsonl event")
            || parse_log.contains("missing terminal agent_message")
    );
}

#[test]
fn provider_timeout_is_logged_and_falls_back_deterministically() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let queue = QueuePaths::from_state_root(&state_root);

    let claude_timeout = dir.path().join("claude-timeout");
    write_script(&claude_timeout, "#!/bin/sh\nsleep 35\necho too-late\n");
    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-timeout"),
        "anthropic",
        1,
    );
    write_incoming(&queue, &sample_message("msg-timeout", "thread-timeout"));

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        1,
        &binaries(claude_timeout.display().to_string(), "unused"),
    )
    .expect("drain timeout fallback");
    assert_eq!(processed, 1);

    let timeout_log = fs::read_to_string(
        state_root.join("orchestrator/select/logs/sel-msg-timeout_attempt_0.invocation.json"),
    )
    .expect("timeout log");
    assert!(timeout_log.contains("\"status\": \"failed\""));
    assert!(timeout_log.contains("\"timedOut\": true"));
    assert!(timeout_log.contains("timed out"));
}

#[test]
fn malformed_queue_payload_is_requeued_not_dropped() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let queue = QueuePaths::from_state_root(&state_root);

    fs::write(queue.incoming.join("malformed.json"), "{not-json").expect("write malformed");

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspace_path: {workspace}
shared_workspaces: {{}}
orchestrators: {{}}
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display()
    ))
    .expect("settings");

    let err =
        drain_queue_once_with_binaries(&state_root, &settings, 1, &binaries("unused", "unused"))
            .expect_err("malformed payload should fail");
    assert!(err.contains("invalid queue payload"));

    let mut incoming_files: Vec<PathBuf> = fs::read_dir(&queue.incoming)
        .expect("incoming")
        .map(|e| e.expect("entry").path())
        .collect();
    incoming_files.sort();
    assert_eq!(incoming_files.len(), 1);
    assert_eq!(
        fs::read_to_string(&incoming_files[0]).expect("requeued malformed"),
        "{not-json"
    );
    assert!(fs::read_dir(&queue.processing)
        .expect("processing")
        .next()
        .is_none());
}

#[test]
fn startup_recovery_moves_processing_back_to_incoming() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let queue = QueuePaths::from_state_root(&state_root);

    fs::write(queue.processing.join("stale.json"), "{\"k\":\"v\"}").expect("stale file");
    let recovered = recover_processing_queue_entries(&state_root).expect("recover");
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].starts_with(&queue.incoming));
    assert_eq!(
        fs::read_to_string(&recovered[0]).expect("recovered content"),
        "{\"k\":\"v\"}"
    );
    assert!(fs::read_dir(&queue.processing)
        .expect("processing")
        .next()
        .is_none());
}

#[test]
fn supervisor_start_recovers_processing_entries_and_processes_message() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let queue = QueuePaths::from_state_root(&state_root);

    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-restart"),
        "anthropic",
        1,
    );
    let stale = sample_message("msg-restart", "thread-restart");
    fs::write(
        queue.processing.join("stale-msg-restart.json"),
        serde_json::to_vec(&stale).expect("serialize stale"),
    )
    .expect("write stale processing");

    let claude = dir.path().join("claude-restart");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-restart\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    let old_anthropic = std::env::var_os("DIRECLAW_PROVIDER_BIN_ANTHROPIC");
    let old_openai = std::env::var_os("DIRECLAW_PROVIDER_BIN_OPENAI");
    std::env::set_var(
        "DIRECLAW_PROVIDER_BIN_ANTHROPIC",
        claude.display().to_string(),
    );
    std::env::set_var("DIRECLAW_PROVIDER_BIN_OPENAI", "unused");

    let state_root_for_thread = state_root.clone();
    let settings_for_thread = settings.clone();
    let handle = thread::spawn(move || run_supervisor(&state_root_for_thread, settings_for_thread));

    let out_dir = state_root.join("queue/outgoing");
    let start = Instant::now();
    while fs::read_dir(&out_dir).expect("outgoing").next().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "runtime did not process recovered queue entry"
        );
        thread::sleep(Duration::from_millis(100));
    }

    signal_stop(&StatePaths::new(&state_root)).expect("signal stop");
    handle
        .join()
        .expect("join supervisor thread")
        .expect("supervisor exit");
    if let Some(value) = old_anthropic {
        std::env::set_var("DIRECLAW_PROVIDER_BIN_ANTHROPIC", value);
    } else {
        std::env::remove_var("DIRECLAW_PROVIDER_BIN_ANTHROPIC");
    }
    if let Some(value) = old_openai {
        std::env::set_var("DIRECLAW_PROVIDER_BIN_OPENAI", value);
    } else {
        std::env::remove_var("DIRECLAW_PROVIDER_BIN_OPENAI");
    }

    let runtime_log = fs::read_to_string(state_root.join("logs/runtime.log")).expect("runtime log");
    assert!(runtime_log.contains("\"event\":\"queue.recovered\""));
    assert!(fs::read_dir(&queue.processing)
        .expect("processing")
        .next()
        .is_none());
}

#[test]
fn queue_runtime_enforces_same_key_ordering_and_cross_key_concurrency() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let queue = QueuePaths::from_state_root(&state_root);
    let orch_ws = dir.path().join("orch-order");
    let settings = write_settings_and_orchestrator(dir.path(), &orch_ws, "anthropic", 1);

    let claude = dir.path().join("claude-order");
    write_script(
        &claude,
        r#"#!/bin/sh
set -eu
line=$(printf "%s\n" "$@" | tr ' ' '\n' | grep -o 'sel-[^/[:space:]]*_attempt_[0-9]*_prompt.md' | head -n1 || true)
selector_id=$(printf "%s" "$line" | sed 's/_attempt_.*$//')
if [ -z "$selector_id" ]; then
  selector_id="unknown"
fi
echo "start $selector_id" >> "$PWD/trace.log"
sleep 1
echo "end $selector_id" >> "$PWD/trace.log"
echo "{\"selectorId\":\"$selector_id\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}"
"#,
    );

    write_incoming(&queue, &sample_message("a1", "thread-a"));
    write_incoming(&queue, &sample_message("a2", "thread-a"));
    write_incoming(&queue, &sample_message("b1", "thread-b"));

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        4,
        &binaries(claude.display().to_string(), "unused"),
    )
    .expect("drain");
    assert_eq!(processed, 3);

    let trace = fs::read_to_string(orch_ws.join("agents/router/trace.log")).expect("trace");
    let lines: Vec<&str> = trace.lines().collect();
    let idx = |needle: &str| -> usize {
        lines
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("missing `{needle}` in trace:\n{trace}"))
    };

    let start_a1 = idx("start sel-a1");
    let end_a1 = idx("end sel-a1");
    let start_a2 = idx("start sel-a2");
    let start_b1 = idx("start sel-b1");

    assert!(start_a1 < end_a1);
    assert!(
        end_a1 < start_a2,
        "same-key messages must be sequential:\n{trace}"
    );
    assert!(
        start_b1 < end_a1,
        "different keys should run concurrently:\n{trace}"
    );
}
