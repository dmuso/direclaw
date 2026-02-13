use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_direclaw"))
        .args(args)
        .env("HOME", home)
        .output()
        .expect("run direclaw")
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

fn write_settings(home: &Path, include_shared_docs: bool) {
    let workspace = home.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let docs = home.join("docs");
    fs::create_dir_all(&docs).expect("create docs");

    let shared = if include_shared_docs {
        format!("  docs: {}", docs.display())
    } else {
        String::new()
    };

    fs::write(
        home.join(".direclaw.yaml"),
        format!(
            r#"
workspace_path: {workspace}
shared_workspaces:
{shared}
orchestrators: {{}}
channel_profiles: {{}}
monitoring:
  heartbeat_interval: 5
channels:
  slack:
    enabled: true
"#,
            workspace = workspace.display(),
            shared = shared,
        ),
    )
    .expect("write settings");
}

fn run_id_from(output: &Output) -> String {
    stdout(output)
        .lines()
        .find_map(|line| line.strip_prefix("run_id=").map(|v| v.to_string()))
        .expect("run id in output")
}

#[test]
fn daemon_command_surface_works() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["setup"]));
    assert_ok(&run(temp.path(), &["start"]));

    let status = run(temp.path(), &["status"]);
    assert_ok(&status);
    assert!(stdout(&status).contains("running=true"));

    assert_ok(&run(temp.path(), &["logs"]));
    assert_ok(&run(temp.path(), &["attach"]));
    assert_ok(&run(temp.path(), &["update", "check"]));
    assert_ok(&run(temp.path(), &["channels", "reset"]));
    assert_ok(&run(temp.path(), &["auth", "sync"]));
    let send_missing_profile = run(temp.path(), &["send", "missing-profile", "hello"]);
    assert_err_contains(&send_missing_profile, "unknown channel profile");

    assert_ok(&run(temp.path(), &["stop"]));
    assert_ok(&run(temp.path(), &["restart"]));
}

#[test]
fn orchestrator_and_agent_commands_work() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));

    let list = run(temp.path(), &["orchestrator", "list"]);
    assert_ok(&list);
    assert!(stdout(&list).contains("alpha"));

    assert_ok(&run(temp.path(), &["orchestrator", "show", "alpha"]));
    assert_ok(&run(
        temp.path(),
        &[
            "orchestrator",
            "set-private-workspace",
            "alpha",
            temp.path().join("alpha-private").to_str().expect("path"),
        ],
    ));
    assert_ok(&run(
        temp.path(),
        &["orchestrator", "grant-shared-access", "alpha", "docs"],
    ));
    assert_ok(&run(
        temp.path(),
        &["orchestrator", "revoke-shared-access", "alpha", "docs"],
    ));
    assert_ok(&run(
        temp.path(),
        &["orchestrator", "set-selection-max-retries", "alpha", "2"],
    ));

    assert_ok(&run(
        temp.path(),
        &["orchestrator-agent", "add", "alpha", "helper"],
    ));
    assert_ok(&run(temp.path(), &["orchestrator-agent", "list", "alpha"]));
    assert_ok(&run(
        temp.path(),
        &["orchestrator-agent", "show", "alpha", "helper"],
    ));
    assert_ok(&run(
        temp.path(),
        &["orchestrator-agent", "reset", "alpha", "helper"],
    ));
    assert_ok(&run(
        temp.path(),
        &["orchestrator-agent", "remove", "alpha", "helper"],
    ));
}

#[test]
fn workflow_commands_work() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));
    assert_ok(&run(temp.path(), &["workflow", "list", "alpha"]));
    assert_ok(&run(temp.path(), &["workflow", "add", "alpha", "triage"]));
    assert_ok(&run(temp.path(), &["workflow", "show", "alpha", "triage"]));
    assert_ok(&run(
        temp.path(),
        &["orchestrator", "set-default-workflow", "alpha", "triage"],
    ));

    let run_output = run(
        temp.path(),
        &[
            "workflow",
            "run",
            "alpha",
            "triage",
            "--input",
            "ticket=123",
        ],
    );
    assert_ok(&run_output);
    let run_id = run_id_from(&run_output);

    assert_ok(&run(temp.path(), &["workflow", "status", &run_id]));
    assert_ok(&run(temp.path(), &["workflow", "progress", &run_id]));
    assert_ok(&run(temp.path(), &["workflow", "cancel", &run_id]));
}

#[test]
fn channel_profile_provider_model_and_agent_alias_work() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));
    assert_ok(&run(
        temp.path(),
        &[
            "channel-profile",
            "add",
            "slack-main",
            "slack",
            "alpha",
            "--slack-app-user-id",
            "U123",
            "--require-mention-in-channels",
            "true",
        ],
    ));
    assert_ok(&run(temp.path(), &["channel-profile", "list"]));
    assert_ok(&run(
        temp.path(),
        &["channel-profile", "show", "slack-main"],
    ));
    assert_ok(&run(
        temp.path(),
        &["channel-profile", "set-orchestrator", "slack-main", "alpha"],
    ));

    assert_ok(&run(
        temp.path(),
        &["provider", "openai", "--model", "gpt-5.2"],
    ));
    assert_ok(&run(temp.path(), &["model", "gpt-5.3-codex"]));
    assert_ok(&run(temp.path(), &["provider"]));

    assert_ok(&run(temp.path(), &["agent", "add", "alpha", "helper"]));
    assert_ok(&run(temp.path(), &["agent", "list", "alpha"]));

    assert_ok(&run(
        temp.path(),
        &["channel-profile", "remove", "slack-main"],
    ));
}

#[test]
fn failure_modes_unknown_orchestrator_invalid_shared_key_and_invalid_workflow_id() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), false);

    let unknown_orch = run(temp.path(), &["workflow", "list", "missing"]);
    assert_err_contains(&unknown_orch, "orchestrator");

    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));

    let invalid_shared = run(
        temp.path(),
        &["orchestrator", "grant-shared-access", "alpha", "missing"],
    );
    assert_err_contains(&invalid_shared, "invalid shared key");

    let invalid_workflow = run(
        temp.path(),
        &[
            "orchestrator",
            "set-default-workflow",
            "alpha",
            "missing-workflow",
        ],
    );
    assert_err_contains(&invalid_workflow, "invalid workflow id");
}
