use serde_json::Value;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

fn run_with_stdin_and_env(
    home: &Path,
    args: &[&str],
    stdin_body: &str,
    envs: &[(&str, &str)],
) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_direclaw"));
    cmd.args(args)
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn direclaw");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin_body.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait output")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn assert_err_contains(output: &Output, needle: &str) {
    assert!(
        !output.status.success(),
        "expected failure, stdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
    let text = format!("{}{}", stdout(output), stderr(output));
    assert!(
        text.contains(needle),
        "expected error to contain `{needle}`, got:\n{text}"
    );
}

fn write_chat_fixture(home: &Path, profile_channel: &str) {
    let workspace = home.join("workspace");
    let orchestrator_workspace = workspace.join("eng_orchestrator");
    fs::create_dir_all(&orchestrator_workspace).expect("create orchestrator workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("create config dir");

    let profile_extras = if profile_channel == "slack" {
        "    slack_app_user_id: U123\n    require_mention_in_channels: true\n"
    } else {
        ""
    };

    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  eng_orchestrator:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles:
  local-default:
    channel: {profile_channel}
    orchestrator_id: eng_orchestrator
{profile_extras}monitoring: {{}}
channels: {{}}
"#,
            workspace = workspace.display(),
            orchestrator_workspace = orchestrator_workspace.display(),
            profile_channel = profile_channel,
            profile_extras = profile_extras,
        ),
    )
    .expect("write settings");

    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
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
        outputs: [summary, artifact]
        output_files:
          summary: outputs/summary.txt
          artifact: outputs/artifact.txt
"#,
    )
    .expect("write orchestrator");
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, body).expect("write mock script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn message_artifact_paths(home: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(home.join(".direclaw/orchestrator/messages"))
        .expect("read orchestrator messages")
        .map(|entry| entry.expect("entry").path())
        .collect();
    files.sort();
    files
}

#[test]
fn chat_multi_turn_persists_messages_under_one_conversation() {
    let temp = tempdir().expect("tempdir");
    write_chat_fixture(temp.path(), "local");

    let claude = temp.path().join("claude-mock");
    let codex = temp.path().join("codex-mock");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-chat\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_script(
        &codex,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );

    let output = run_with_stdin_and_env(
        temp.path(),
        &["chat", "local-default"],
        "hello\nsecond turn\n/exit\n",
        &[
            (
                "DIRECLAW_PROVIDER_BIN_ANTHROPIC",
                claude.to_str().expect("claude str"),
            ),
            (
                "DIRECLAW_PROVIDER_BIN_OPENAI",
                codex.to_str().expect("codex str"),
            ),
        ],
    );
    assert_ok(&output);

    let out = stdout(&output);
    assert_eq!(out.matches("assistant> ok").count(), 2);
    assert!(!out.contains("assistant> workflow started"));
    let conversation_id = out
        .lines()
        .find_map(|line| line.strip_prefix("conversation_id="))
        .expect("conversation id line")
        .to_string();
    assert!(conversation_id.starts_with("chat-"));

    let files = message_artifact_paths(temp.path());
    assert_eq!(files.len(), 2, "expected two persisted inbound messages");

    let mut messages = Vec::new();
    let mut conversation_ids = Vec::new();
    for path in files {
        let payload: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read payload"))
                .expect("parse payload");
        messages.push(
            payload
                .get("message")
                .and_then(Value::as_str)
                .expect("message")
                .to_string(),
        );
        conversation_ids.push(
            payload
                .get("conversationId")
                .and_then(Value::as_str)
                .expect("conversationId")
                .to_string(),
        );
    }

    assert!(messages.contains(&"hello".to_string()));
    assert!(messages.contains(&"second turn".to_string()));
    assert_eq!(conversation_ids[0], conversation_id);
    assert_eq!(conversation_ids[1], conversation_id);
}

#[test]
fn chat_rejects_non_local_channel_profile() {
    let temp = tempdir().expect("tempdir");
    write_chat_fixture(temp.path(), "slack");

    let output = run_with_stdin_and_env(temp.path(), &["chat", "local-default"], "hello\n", &[]);
    assert_err_contains(&output, "chat requires a local channel profile");
}
