use direclaw::config::Settings;
use direclaw::orchestration::run_store::{RunState, WorkflowRunStore};
use direclaw::provider::RunnerBinaries;
use direclaw::queue::{IncomingMessage, OutgoingMessage, QueuePaths};
use direclaw::runtime::{
    bootstrap_state_root, drain_queue_once_with_binaries, recover_processing_queue_entries,
    run_supervisor, signal_stop, StatePaths,
};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

fn write_script(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn write_openai_success_script(path: &Path) {
    write_script(
        path,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );
}

fn write_selector_workflow_status_script(path: &Path) {
    write_script(
        path,
        r#"#!/bin/sh
set -eu
msg="$*"
result_path=$(printf "%s" "$msg" | sed -n 's/.*Write selector result JSON to: \([^ ]*\).*/\1/p')
if [ -z "$result_path" ]; then
  echo "missing selector result path" 1>&2
  exit 1
fi
selector_id=$(basename "$result_path" | sed -E 's/^selector-provider-result-(.*)_attempt_[0-9]+\.json$/\1/')
printf '{"selectorId":"%s","status":"selected","action":"workflow_status"}' "$selector_id" > "$result_path"
echo "ok"
"#,
    );
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
        is_direct: true,
        is_thread_reply: true,
        is_mentioned: false,
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    }
}

fn write_incoming(queue: &QueuePaths, payload: &IncomingMessage) {
    fs::create_dir_all(&queue.incoming).expect("incoming dir");
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
    selector_timeout_seconds: u64,
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
selector_timeout_seconds: {selector_timeout_seconds}
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
    description: default triage workflow
    tags: [triage, default]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
        outputs: [summary, artifact]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#
        ),
    )
    .expect("write orchestrator");

    serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
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

fn write_memory_enabled_settings_with_two_profiles(
    temp_root: &Path,
    orchestrator_workspace: &Path,
) -> Settings {
    fs::create_dir_all(orchestrator_workspace).expect("orchestrator workspace");
    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
        r#"
id: eng_orchestrator
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
    description: memory-aware triage workflow
    tags: [triage, memory]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: "memory_context={{workflow.memory_context_bulletin}}"
        outputs: [summary, artifact]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("write orchestrator");

    serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
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
  support:
    channel: slack
    orchestrator_id: eng_orchestrator
    slack_app_user_id: U456
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
memory:
  enabled: true
  bulletin_mode: every_message
  retrieval:
    top_n: 20
    rrf_k: 60
  ingest:
    enabled: true
    max_file_size_mb: 25
  scope:
    cross_orchestrator: false
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

fn queue_for_profile(settings: &Settings, profile_id: &str) -> QueuePaths {
    let runtime_root = settings
        .resolve_channel_profile_runtime_root(profile_id)
        .expect("resolve runtime root");
    QueuePaths::from_state_root(&runtime_root)
}

fn queue_for_orchestrator(settings: &Settings, orchestrator_id: &str) -> QueuePaths {
    let runtime_root = settings
        .resolve_orchestrator_runtime_root(orchestrator_id)
        .expect("resolve runtime root");
    QueuePaths::from_state_root(&runtime_root)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn read_outgoing_text(queue: &QueuePaths) -> String {
    let out_dir = queue.outgoing.clone();
    let mut files: Vec<PathBuf> = fs::read_dir(&out_dir)
        .expect("read outgoing")
        .map(|e| e.expect("entry").path())
        .collect();
    files.sort();
    let path = files.pop().expect("outgoing file");
    fs::read_to_string(path).expect("outgoing text")
}

fn read_outgoing_messages(queue: &QueuePaths) -> Vec<OutgoingMessage> {
    let out_dir = queue.outgoing.clone();
    let mut files: Vec<PathBuf> = fs::read_dir(&out_dir)
        .expect("read outgoing")
        .map(|e| e.expect("entry").path())
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            serde_json::from_str(&fs::read_to_string(path).expect("outgoing payload"))
                .expect("parse outgoing payload")
        })
        .collect()
}

fn latest_run_dir(queue: &QueuePaths) -> PathBuf {
    let run_root = queue.root.join("workflows/runs");
    let mut run_dirs: Vec<PathBuf> = fs::read_dir(&run_root)
        .expect("run root")
        .map(|e| e.expect("entry").path())
        .filter(|path| path.is_dir())
        .collect();
    run_dirs.sort();
    run_dirs.pop().expect("run dir")
}

#[test]
fn queue_to_orchestrator_runtime_path_runs_provider_and_persists_selector_artifacts() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-mock");
    let codex = dir.path().join("codex-mock");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-1\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_openai_success_script(&codex);

    let settings =
        write_settings_and_orchestrator(dir.path(), &dir.path().join("orch"), "anthropic", 1, 30);
    let queue = queue_for_profile(&settings, "eng");
    write_incoming(&queue, &sample_message("msg-1", "thread-1"));

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        4,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain");
    assert_eq!(processed, 1);

    let outgoing = read_outgoing_text(&queue);
    assert!(outgoing.contains("ok"));
    assert!(queue
        .root
        .join("orchestrator/artifacts/message-msg-1.json")
        .is_file());
    assert!(queue
        .root
        .join("orchestrator/artifacts/selector-result-sel-msg-1.json")
        .is_file());
    assert!(!queue
        .root
        .join("orchestrator/select/logs/sel-msg-1_attempt_0.invocation.json")
        .is_file());
    let selector_result = fs::read_to_string(
        queue
            .root
            .join("orchestrator/artifacts/selector-result-sel-msg-1.json"),
    )
    .expect("selector result");
    assert!(selector_result.contains("fallback_to_default_workflow_after_retry_limit"));
}

#[test]
fn selector_no_response_produces_no_outgoing() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-selector");
    let codex = dir.path().join("codex-worker");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-low-value\",\"status\":\"selected\",\"action\":\"no_response\",\"reason\":\"context_only\"}'\n",
    );
    write_script(
        &codex,
        r#"#!/bin/sh
echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"Nice catch. AccountEmailDomain::Verify is taking around 20-30 seconds due to DKIM and DMARC DNS checks. For next iteration we could make this async and return 202 with verification_pending.\"}[/workflow_result]"}}'
"#,
    );

    let mut settings =
        write_settings_and_orchestrator(dir.path(), &dir.path().join("orch"), "anthropic", 1, 30);
    settings
        .channel_profiles
        .get_mut("eng")
        .expect("eng profile")
        .thread_response_mode = direclaw::config::ThreadResponseMode::SelectiveReply;
    let queue = queue_for_profile(&settings, "eng");
    let mut inbound = sample_message("msg-low-value", "C111:1700000000.1");
    inbound.is_direct = false;
    inbound.message = "One thing I noticed: AccountEmailDomain::Verify is taking ~20-30 seconds because of DKIM/DMARC DNS checks. For next iteration we could make this async and return 202 verification_pending.".to_string();
    write_incoming(&queue, &inbound);

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        4,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain");
    assert_eq!(processed, 1);
    assert_eq!(
        fs::read_dir(&queue.outgoing)
            .expect("outgoing list")
            .count(),
        0,
        "selector no_response should produce no outgoing files"
    );
}

#[test]
fn selector_workflow_start_produces_outgoing() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-selector-local");
    let codex = dir.path().join("codex-worker-local");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-local-low-value\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_script(
        &codex,
        r#"#!/bin/sh
echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"Nice catch. AccountEmailDomain::Verify is taking around 20-30 seconds due to DKIM and DMARC DNS checks. For next iteration we could make this async and return 202 with verification_pending.\"}[/workflow_result]"}}'
"#,
    );

    let orchestrator_workspace = dir.path().join("orch-local");
    let settings = serde_yaml::from_str::<Settings>(&format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  eng_orchestrator:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles:
  local-eng:
    channel: local
    orchestrator_id: eng_orchestrator
    thread_response_mode: selective_reply
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display(),
        orchestrator_workspace = orchestrator_workspace.display()
    ))
    .expect("settings");
    fs::create_dir_all(&orchestrator_workspace).expect("orchestrator workspace");
    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
        r#"
id: eng_orchestrator
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
    description: default triage workflow
    tags: [triage, default]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
        outputs: [summary, artifact]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("write orchestrator");

    let queue = queue_for_profile(&settings, "local-eng");
    let inbound = IncomingMessage {
        channel: "local".to_string(),
        channel_profile_id: Some("local-eng".to_string()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: "One thing I noticed: AccountEmailDomain::Verify is taking ~20-30 seconds because of DKIM/DMARC DNS checks. For next iteration we could make this async and return 202 verification_pending.".to_string(),
        timestamp: 100,
        message_id: "msg-local-low-value".to_string(),
        conversation_id: Some("chat-1".to_string()),
        is_direct: true,
        is_thread_reply: true,
        is_mentioned: false,
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    write_incoming(&queue, &inbound);

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        4,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain");
    assert_eq!(processed, 1);
    assert!(
        fs::read_dir(&queue.outgoing)
            .expect("outgoing list")
            .count()
            >= 1,
        "workflow_start should produce at least one outgoing message"
    );
}

#[test]
fn memory_enabled_cross_channel_recall_changes_workflow_output() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-selector-memory");
    let codex = dir.path().join("codex-worker-memory");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-memory\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_script(
        &codex,
        r#"#!/bin/sh
set -eu
args="$*"
if printf "%s" "$args" | grep -q "transcript-msg-memory-a"; then
  echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"summary\":\"memory_hit\",\"artifact\":\"memory_hit\"}[/workflow_result]"}}'
else
  echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"summary\":\"memory_miss\",\"artifact\":\"memory_miss\"}[/workflow_result]"}}'
fi
"#,
    );

    let settings = write_memory_enabled_settings_with_two_profiles(
        dir.path(),
        &dir.path().join("orch-memory"),
    );
    let eng_queue = queue_for_profile(&settings, "eng");
    let support_queue = queue_for_profile(&settings, "support");

    let mut first = sample_message("msg-memory-a", "thread-eng");
    first.message = "projectcycloneanchor".to_string();
    write_incoming(&eng_queue, &first);
    let processed_first = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        4,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain first");
    assert_eq!(processed_first, 1);

    let mut second = sample_message("msg-memory-b", "thread-support");
    second.channel_profile_id = Some("support".to_string());
    second.message = "projectcycloneanchor".to_string();
    write_incoming(&support_queue, &second);
    let processed_second = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        4,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain second");
    assert_eq!(processed_second, 1);

    let second_run_dir = latest_run_dir(&support_queue);
    let prompt_path = second_run_dir.join("steps/start/attempts/1/prompt.md");
    let prompt_text = fs::read_to_string(prompt_path).expect("read step prompt");
    assert!(
        prompt_text.contains("projectcycloneanchor"),
        "expected workflow step prompt to include recalled memory context; prompt={prompt_text}"
    );
    assert!(
        prompt_text.contains("transcript-msg-memory-a"),
        "expected workflow step prompt to include citation-ready memory id; prompt={prompt_text}"
    );
}

#[test]
fn channel_ingress_multi_step_workflow_reaches_terminal_state_and_writes_safe_outputs() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-multi-selector");
    let codex = dir.path().join("codex-multi-worker");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-multi\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_script(
        &codex,
        r#"#!/bin/sh
set -eu
args="$*"
if printf "%s" "$args" | grep -q "/steps/plan/"; then
  echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"plan\":\"Plan: inspect logs\",\"summary\":\"Summary: collect traces\"}[/workflow_result]"}}'
elif printf "%s" "$args" | grep -q "/steps/review/"; then
  echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"decision\":\"approve\",\"summary\":\"approved\",\"feedback\":\"none\"}[/workflow_result]"}}'
else
  echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"summary\":\"completed\",\"result\":{\"status\":\"done\",\"ticket\":\"123\"}}[/workflow_result]"}}'
fi
"#,
    );

    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-multi"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings, "eng");
    fs::write(
        dir.path().join("orch-multi/orchestrator.yaml"),
        r#"
id: eng_orchestrator
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
    description: triage with review loop
    tags: [triage, review]
    inputs: [ticket]
    limits:
      max_total_iterations: 6
      run_timeout_seconds: 40
    steps:
      - id: plan
        type: agent_task
        agent: worker
        prompt: plan message={{inputs.user_message}}
        outputs: [plan, summary]
        output_files:
          plan: artifacts/plan.md
          summary: artifacts/summary.txt
        next: review
      - id: review
        type: agent_review
        agent: worker
        prompt: review run={{workflow.run_id}}
        outputs: [decision, summary, feedback]
        output_files:
          decision: artifacts/review-decision.txt
          summary: artifacts/review-summary.txt
          feedback: artifacts/review-feedback.txt
        on_approve: done
        on_reject: plan
      - id: done
        type: agent_task
        agent: worker
        prompt: finalize
        outputs: [summary, result]
        output_files:
          summary: artifacts/done-summary.txt
          result: artifacts/result.json
"#,
    )
    .expect("write orchestrator");
    let mut inbound = sample_message("msg-multi", "thread-multi");
    inbound.message = "ship this".to_string();
    write_incoming(&queue, &inbound);

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        4,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain");
    assert_eq!(processed, 1);

    let outgoing_messages = read_outgoing_messages(&queue);
    assert!(
        outgoing_messages
            .iter()
            .all(|message| !message.message.contains("Running step")),
        "did not expect running-step lifecycle updates in outbound messages"
    );
    assert!(
        outgoing_messages
            .iter()
            .all(|message| !message.message.contains("Step `")),
        "did not expect step-complete lifecycle updates in outbound messages"
    );
    assert!(
        outgoing_messages.iter().any(
            |message| message.message.contains("Summary: collect traces")
                || message.message.contains("approved")
                || message.message.contains("completed")
        ),
        "expected final user-facing summary in outbound messages"
    );
    let mut run_ids: Vec<String> = fs::read_dir(queue.root.join("workflows/runs"))
        .expect("run dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().and_then(|v| v.to_str()) == Some("json"))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|v| v.to_str())
                .map(|v| v.to_string())
        })
        .collect();
    run_ids.sort();
    let run_id = run_ids.pop().expect("run id");

    let run_root = queue.root.join(format!("workflows/runs/{run_id}"));
    assert!(queue
        .root
        .join(format!("workflows/runs/{run_id}.json"))
        .is_file());
    assert!(run_root.join("progress.json").is_file());
    assert!(queue.root.join("logs/orchestrator.log").is_file());
    let progress = fs::read_to_string(run_root.join("progress.json")).expect("progress");
    assert!(progress.contains("\"state\": \"succeeded\""));
    let engine_log =
        fs::read_to_string(queue.root.join("logs/orchestrator.log")).expect("orchestrator log");
    assert!(engine_log.contains("transition=succeeded"));

    for step in ["plan", "review", "done"] {
        assert!(run_root
            .join(format!("steps/{step}/attempts/1/result.json"))
            .is_file());
    }

    let plan_output = run_root.join("steps/plan/attempts/1/artifacts/plan.md");
    let summary_output = run_root.join("steps/plan/attempts/1/artifacts/summary.txt");
    let result_output = run_root.join("steps/done/attempts/1/artifacts/result.json");
    for output in [&plan_output, &summary_output, &result_output] {
        assert!(output.is_file(), "missing output file {}", output.display());
        let canonical = fs::canonicalize(output).expect("canonical output path");
        assert!(
            canonical.starts_with(&run_root),
            "output escaped run root: {}",
            canonical.display()
        );
    }
    assert_eq!(
        fs::read_to_string(&plan_output).expect("plan output"),
        "Plan: inspect logs"
    );
    assert_eq!(
        fs::read_to_string(&summary_output).expect("summary output"),
        "Summary: collect traces"
    );
    assert!(fs::read_to_string(&result_output)
        .expect("result output")
        .contains("\"status\": \"done\""));

    let mut status_request = sample_message("msg-multi-status", "thread-multi");
    status_request.workflow_run_id = Some(run_id.clone());
    status_request.message = "/status".to_string();
    write_incoming(&queue, &status_request);
    let claude_status = dir.path().join("claude-multi-selector-status");
    write_selector_workflow_status_script(&claude_status);

    let status_processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        2,
        &binaries(
            claude_status.display().to_string(),
            codex.display().to_string(),
        ),
    )
    .expect("drain status");
    assert_eq!(status_processed, 1);

    let status_outbound = read_outgoing_messages(&queue)
        .into_iter()
        .find(|outbound| outbound.message_id == "msg-multi-status")
        .expect("status outbound message");
    assert_eq!(
        status_outbound.workflow_run_id.as_deref(),
        Some(run_id.as_str())
    );
    assert!(status_outbound.message.contains("workflow progress loaded"));
    assert!(status_outbound.message.contains("state=succeeded"));
}

#[test]
fn workflow_bound_message_resumes_run_execution_when_not_status_command() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-mock");
    let codex = dir.path().join("codex-mock");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"unused\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_openai_success_script(&codex);

    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-resume"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings, "eng");
    let run_store = WorkflowRunStore::new(&queue.root);
    run_store
        .create_run("run-resume-1", "triage", now_secs())
        .expect("create run");
    let mut inbound = sample_message("msg-resume", "thread-resume");
    inbound.workflow_run_id = Some("run-resume-1".to_string());
    inbound.message = "please continue".to_string();
    write_incoming(&queue, &inbound);

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        2,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain");
    assert_eq!(processed, 1);

    let run = run_store.load_run("run-resume-1").expect("load run");
    assert_eq!(run.state, RunState::Succeeded);
    assert!(queue
        .root
        .join("workflows/runs/run-resume-1/steps/start/attempts/1/result.json")
        .is_file());
    let outgoing = read_outgoing_text(&queue);
    assert!(outgoing.contains("workflow progress loaded"));
}

#[test]
fn workflow_bound_status_command_is_read_only_and_does_not_advance_steps() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let claude_status = dir.path().join("claude-status-only-selector");
    write_selector_workflow_status_script(&claude_status);

    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-status-only"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings, "eng");
    let run_store = WorkflowRunStore::new(&queue.root);
    let mut run = run_store
        .create_run("run-status-1", "triage", 1)
        .expect("create run");
    run_store
        .transition_state(
            &mut run,
            RunState::Running,
            2,
            "running",
            false,
            "execute next step",
        )
        .expect("transition");

    let mut inbound = sample_message("msg-status-only", "thread-status");
    inbound.workflow_run_id = Some("run-status-1".to_string());
    inbound.message = "/status".to_string();
    write_incoming(&queue, &inbound);

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        1,
        &binaries(claude_status.display().to_string(), "unused"),
    )
    .expect("drain");
    assert_eq!(processed, 1);

    assert!(!state_root
        .join("workflows/runs/run-status-1/steps/start/attempts/1/result.json")
        .exists());
    let outgoing = read_outgoing_text(&queue);
    assert!(outgoing.contains("workflow progress loaded"));
}

#[test]
fn workflow_bound_message_with_unknown_run_id_does_not_requeue_forever() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-mock");
    let codex = dir.path().join("codex-mock");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"unused\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_openai_success_script(&codex);

    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-missing-run"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings, "eng");
    let mut inbound = sample_message("msg-missing-run", "thread-missing-run");
    inbound.workflow_run_id = Some("run-does-not-exist".to_string());
    inbound.message = "continue please".to_string();
    write_incoming(&queue, &inbound);

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        2,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain");
    assert_eq!(processed, 1);

    let outgoing = read_outgoing_text(&queue);
    assert!(outgoing.contains("workflow run `run-does-not-exist` was not found"));
    assert!(fs::read_dir(&queue.processing)
        .expect("processing")
        .next()
        .is_none());
    assert!(fs::read_dir(&queue.incoming)
        .expect("incoming")
        .next()
        .is_none());

    let processed_again = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        2,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain second");
    assert_eq!(processed_again, 0);
}

#[test]
fn queue_failures_requeue_without_payload_loss_for_unknown_profile() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  eng_orchestrator:
    private_workspace: {workspace}/orch-unknown-profile
    shared_access: []
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display()
    ))
    .expect("settings");
    let queue = queue_for_orchestrator(&settings, "eng_orchestrator");
    fs::create_dir_all(&queue.incoming).expect("incoming");

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
fn runtime_logs_and_persisted_progress_expose_failure_reason_for_limit_triggers() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-reject");
    let codex = dir.path().join("codex-ok");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-limit\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_openai_success_script(&codex);
    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-limit"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings, "eng");

    fs::write(
        dir.path().join("orch-limit/orchestrator.yaml"),
        r#"
id: eng_orchestrator
selector_agent: router
default_workflow: triage
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: anthropic
    model: sonnet
workflows:
  - id: triage
    version: 1
    description: triage workflow for limit validation
    tags: [triage, limits]
    limits:
      max_total_iterations: 1
    steps:
      - id: review
        type: agent_review
        agent: worker
        prompt: review
        outputs: [decision, summary, feedback]
        output_files:
          decision: outputs/{{workflow.step_id}}-{{workflow.attempt}}-decision.txt
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          feedback: outputs/{{workflow.step_id}}-{{workflow.attempt}}-feedback.txt
        on_approve: done
        on_reject: review
      - id: done
        type: agent_task
        agent: worker
        prompt: done
        outputs: [summary, artifact]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("overwrite orchestrator");
    write_script(
        &codex,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"decision\\\":\\\"reject\\\",\\\"summary\\\":\\\"need more\\\",\\\"feedback\\\":\\\"retry\\\"}[/workflow_result]\"}}'\n",
    );

    write_incoming(&queue, &sample_message("msg-limit", "thread-limit"));
    let err = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        1,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect_err("limit failure should bubble to queue worker");
    assert!(err.contains("max total iterations"));

    let mut incoming_files: Vec<PathBuf> = fs::read_dir(&queue.incoming)
        .expect("incoming")
        .map(|e| e.expect("entry").path())
        .collect();
    incoming_files.sort();
    assert_eq!(incoming_files.len(), 1);
    let requeued = fs::read_to_string(&incoming_files[0]).expect("requeued");
    assert!(requeued.contains("\"messageId\":\"msg-limit\""));

    let run_root = queue.root.join("workflows/runs");
    let mut run_dirs: Vec<PathBuf> = fs::read_dir(&run_root)
        .expect("run root")
        .map(|e| e.expect("entry").path())
        .filter(|path| path.is_dir())
        .collect();
    run_dirs.sort();
    let run_dir = run_dirs.pop().expect("run dir");
    let progress = fs::read_to_string(run_dir.join("progress.json")).expect("read progress");
    assert!(progress.contains("\"state\": \"failed\""));
    assert!(progress.contains("max total iterations"));
    let engine_log =
        fs::read_to_string(queue.root.join("logs/orchestrator.log")).expect("orchestrator log");
    assert!(engine_log.contains("transition=failed"));
}

#[test]
fn provider_non_zero_and_parse_failures_are_logged_and_fall_back_deterministically() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude_ok = dir.path().join("claude-ok");
    let codex_fail = dir.path().join("codex-fail");
    write_script(
        &claude_ok,
        "#!/bin/sh\necho '{\"selectorId\":\"unused\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_script(&codex_fail, "#!/bin/sh\necho fail 1>&2\nexit 7\n");
    let settings_fail = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-fail"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings_fail, "eng");
    write_incoming(&queue, &sample_message("msg-fail", "thread-1"));
    let err = drain_queue_once_with_binaries(
        &state_root,
        &settings_fail,
        2,
        &binaries(
            claude_ok.display().to_string(),
            codex_fail.display().to_string(),
        ),
    )
    .expect_err("drain non-zero");
    assert!(err.to_string().contains("exit code 7"));
    let non_zero_log = fs::read_to_string(
        latest_run_dir(&queue).join("steps/start/attempts/1/provider_invocation.json"),
    )
    .expect("non-zero log");
    assert!(non_zero_log.contains("\"provider\": \"openai\""));
    assert!(non_zero_log.contains("\"exitCode\": 7"));

    let codex_bad = dir.path().join("codex-bad");
    write_script(
        &codex_bad,
        r#"#!/bin/sh
set -eu
args="$*"
echo '{not-json}'
"#,
    );
    let settings_parse = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-parse"),
        "openai",
        1,
        30,
    );
    let queue_parse = queue_for_profile(&settings_parse, "eng");
    write_incoming(&queue_parse, &sample_message("msg-parse", "thread-2"));
    let err = drain_queue_once_with_binaries(
        &state_root,
        &settings_parse,
        2,
        &binaries(
            claude_ok.display().to_string(),
            codex_bad.display().to_string(),
        ),
    )
    .expect_err("drain parse failure");
    assert!(err.to_string().contains("output parse failure"));
    let parse_log = fs::read_to_string(
        latest_run_dir(&queue_parse).join("steps/start/attempts/1/provider_invocation.json"),
    )
    .expect("parse log");
    assert!(parse_log.contains("\"provider\": \"openai\""));
    assert!(parse_log.contains("\"timedOut\": false"));
}

#[test]
fn provider_timeout_is_logged_and_falls_back_deterministically() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-timeout-ok");
    let codex_timeout = dir.path().join("codex-timeout");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"unused\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_script(&codex_timeout, "#!/bin/sh\nwhile :; do :; done\n");
    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-timeout"),
        "anthropic",
        1,
        1,
    );
    fs::write(
        dir.path().join("orch-timeout/orchestrator.yaml"),
        r#"
id: eng_orchestrator
selector_agent: router
default_workflow: triage
selection_max_retries: 1
selector_timeout_seconds: 1
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
    description: default triage workflow
    tags: [triage, default]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
        outputs: [summary, artifact]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
workflow_orchestration:
  default_step_timeout_seconds: 0
  max_step_timeout_seconds: 0
"#,
    )
    .expect("rewrite orchestrator for step timeout");
    let queue = queue_for_profile(&settings, "eng");
    write_incoming(&queue, &sample_message("msg-timeout", "thread-timeout"));

    let err = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        1,
        &binaries(
            claude.display().to_string(),
            codex_timeout.display().to_string(),
        ),
    )
    .expect_err("drain timeout failure");
    assert!(err.to_string().contains("timed out"));

    let timeout_log = fs::read_to_string(
        latest_run_dir(&queue).join("steps/start/attempts/1/provider_invocation.json"),
    )
    .expect("timeout log");
    assert!(timeout_log.contains("\"provider\": \"openai\""));
    assert!(timeout_log.contains("\"timedOut\": true"));
}

#[test]
fn malicious_output_file_template_is_rejected_and_security_log_records_denial() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let claude = dir.path().join("claude-malicious-selector");
    let codex = dir.path().join("codex-malicious-worker");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-malicious\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_openai_success_script(&codex);
    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-malicious"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings, "eng");
    fs::write(
        dir.path().join("orch-malicious/orchestrator.yaml"),
        r#"
id: eng_orchestrator
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
    description: triage workflow for output path validation
    tags: [triage, security]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
        outputs: [summary, result]
        output_files:
          summary: outputs/summary.txt
          result: ../../escape.md
"#,
    )
    .expect("write malicious orchestrator");

    write_incoming(&queue, &sample_message("msg-malicious", "thread-malicious"));
    let err = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        1,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect_err("malicious output file template must fail");
    assert!(
        err.contains("requires `summary` mapping in `output_files`")
            || err.contains("output path validation failed")
    );

    let security_log =
        fs::read_to_string(queue.root.join("logs/orchestrator.log")).expect("orchestrator log");
    assert!(security_log.contains("output path validation denied"));
    assert!(security_log.contains("sel-msg-malicious"));
    assert!(fs::read_dir(&queue.processing)
        .expect("processing")
        .next()
        .is_none());
}

#[test]
fn malformed_queue_payload_is_requeued_not_dropped() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  eng_orchestrator:
    private_workspace: {workspace}/orch-malformed
    shared_access: []
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display()
    ))
    .expect("settings");
    let queue = queue_for_orchestrator(&settings, "eng_orchestrator");
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::write(queue.incoming.join("malformed.json"), "{not-json").expect("write malformed");

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
    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-recovery"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings, "eng");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::write(queue.processing.join("stale.json"), "{\"k\":\"v\"}").expect("stale file");
    let recovered = recover_processing_queue_entries(&queue.root).expect("recover");
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
fn recovered_workflow_bound_message_resumes_existing_run_after_restart() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-recovered-resume"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings, "eng");

    let run_store = WorkflowRunStore::new(&queue.root);
    run_store
        .create_run("run-recovered-1", "triage", now_secs())
        .expect("create run");

    let mut payload = sample_message("msg-recovered-resume", "thread-recovered-resume");
    payload.workflow_run_id = Some("run-recovered-1".to_string());
    payload.message = "continue".to_string();
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::write(
        queue.processing.join("stale-workflow-bound.json"),
        serde_json::to_vec(&payload).expect("serialize"),
    )
    .expect("write stale processing");

    let recovered = recover_processing_queue_entries(&queue.root).expect("recover");
    assert_eq!(recovered.len(), 1);

    let claude = dir.path().join("claude-recovered-resume");
    let codex = dir.path().join("codex-recovered-resume");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"unused\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_openai_success_script(&codex);

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        1,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain");
    assert_eq!(processed, 1);

    let run = run_store.load_run("run-recovered-1").expect("run");
    assert_eq!(run.state, RunState::Succeeded);
    assert!(queue
        .root
        .join("workflows/runs/run-recovered-1/steps/start/attempts/1/result.json")
        .is_file());
}

#[test]
fn supervisor_start_recovers_processing_entries_and_processes_message() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");
    let settings = write_settings_and_orchestrator(
        dir.path(),
        &dir.path().join("orch-restart"),
        "anthropic",
        1,
        30,
    );
    let queue = queue_for_profile(&settings, "eng");
    fs::create_dir_all(&queue.incoming).expect("incoming");
    let stale = sample_message("msg-restart", "thread-restart");
    fs::create_dir_all(&queue.processing).expect("processing");
    fs::write(
        queue.processing.join("stale-msg-restart.json"),
        serde_json::to_vec(&stale).expect("serialize stale"),
    )
    .expect("write stale processing");

    let claude = dir.path().join("claude-restart");
    let codex = dir.path().join("codex-restart");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-restart\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_openai_success_script(&codex);
    let old_anthropic = std::env::var_os("DIRECLAW_PROVIDER_BIN_ANTHROPIC");
    let old_openai = std::env::var_os("DIRECLAW_PROVIDER_BIN_OPENAI");
    std::env::set_var(
        "DIRECLAW_PROVIDER_BIN_ANTHROPIC",
        claude.display().to_string(),
    );
    std::env::set_var("DIRECLAW_PROVIDER_BIN_OPENAI", codex.display().to_string());

    let state_root_for_thread = state_root.clone();
    let settings_for_thread = settings.clone();
    let handle = thread::spawn(move || run_supervisor(&state_root_for_thread, settings_for_thread));

    let out_dir = queue.outgoing.clone();
    fs::create_dir_all(&out_dir).expect("outgoing");
    let start = Instant::now();
    while fs::read_dir(&out_dir).expect("outgoing").next().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "runtime did not process recovered queue entry"
        );
        thread::sleep(Duration::from_millis(20));
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
    let orch_ws = dir.path().join("orch-order");
    let settings = write_settings_and_orchestrator(dir.path(), &orch_ws, "anthropic", 1, 30);
    let queue = queue_for_profile(&settings, "eng");

    let claude = dir.path().join("claude-order");
    let codex = dir.path().join("codex-order");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"unused\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_script(
        &codex,
        r#"#!/bin/sh
set -eu
line=$(printf "%s\n" "$@" | tr ' ' '\n' | grep -oE 'run-[0-9a-z]+-[0-9a-z]{4}' | head -n1 || true)
run_id=$(printf "%s" "$line")
if [ -z "$run_id" ]; then
  run_id="unknown"
fi
echo "start $run_id" >> "$PWD/trace.log"
sleep 0.08
echo "end $run_id" >> "$PWD/trace.log"
echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"ok\"}[/workflow_result]"}}'
"#,
    );

    write_incoming(&queue, &sample_message("a1", "thread-a"));
    write_incoming(&queue, &sample_message("a2", "thread-a"));
    write_incoming(&queue, &sample_message("b1", "thread-b"));

    let processed = drain_queue_once_with_binaries(
        &state_root,
        &settings,
        4,
        &binaries(claude.display().to_string(), codex.display().to_string()),
    )
    .expect("drain");
    assert_eq!(processed, 3);

    let trace = fs::read_to_string(orch_ws.join("trace.log")).expect("trace");
    let lines: Vec<&str> = trace.lines().collect();
    let idx = |needle: &str| -> usize {
        lines
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("missing `{needle}` in trace:\n{trace}"))
    };

    let mut run_id_by_message = BTreeMap::new();
    for entry in fs::read_dir(queue.root.join("workflows/runs")).expect("runs dir") {
        let entry = entry.expect("run entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read(&path).expect("read run record");
        let value: serde_json::Value = serde_json::from_slice(&raw).expect("parse run record");
        let Some(run_id) = value.get("runId").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(source_message_id) = value
            .get("sourceMessageId")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        if source_message_id == "a1" || source_message_id == "a2" || source_message_id == "b1" {
            run_id_by_message.insert(source_message_id.to_string(), run_id.to_string());
        }
    }
    assert_eq!(run_id_by_message.len(), 3, "expected run IDs for a1/a2/b1");

    let run_a1 = run_id_by_message.get("a1").expect("run id for a1");
    let run_a2 = run_id_by_message.get("a2").expect("run id for a2");
    let run_b1 = run_id_by_message.get("b1").expect("run id for b1");
    let start_a1 = idx(&format!("start {run_a1}"));
    let end_a1 = idx(&format!("end {run_a1}"));
    let start_a2 = idx(&format!("start {run_a2}"));
    let start_b1 = idx(&format!("start {run_b1}"));

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
