use direclaw::config::OrchestratorConfig;
use std::collections::BTreeMap;
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

fn run_with_env(home: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_direclaw"));
    cmd.args(args).env("HOME", home);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("run direclaw")
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

fn kv_lines(output: &Output) -> BTreeMap<String, String> {
    stdout(output)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn write_settings(home: &Path, include_shared_workspace: bool) {
    let workspace = home.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("create config dir");
    let shared_workspace = home.join("shared-workspace");
    fs::create_dir_all(&shared_workspace).expect("create shared workspace");

    let shared = if include_shared_workspace {
        format!("  shared: {}", shared_workspace.display())
    } else {
        String::new()
    };

    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspaces_path: {workspace}
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
    let status_contract = kv_lines(&status);
    for required in [
        "ownership",
        "running",
        "pid",
        "started_at",
        "stopped_at",
        "last_error",
    ] {
        assert!(
            status_contract.contains_key(required),
            "status output missing `{required}`:\n{}",
            stdout(&status)
        );
    }

    assert_ok(&run(temp.path(), &["logs"]));
    assert_ok(&run(temp.path(), &["attach"]));
    let update = run_with_env(
        temp.path(),
        &["update", "check"],
        &[
            ("DIRECLAW_UPDATE_API_URL", "http://127.0.0.1:1"),
            ("DIRECLAW_UPDATE_REPO", "dharper/rustyclaw"),
        ],
    );
    assert_err_contains(&update, "update check failed");
    assert_err_contains(
        &update,
        "remediation: verify network access and set DIRECLAW_UPDATE_REPO/DIRECLAW_UPDATE_API_URL if needed",
    );
    assert_ok(&run(temp.path(), &["doctor"]));
    assert_ok(&run(temp.path(), &["channels", "reset"]));
    assert_ok(&run(temp.path(), &["auth", "sync"]));
    let send_missing_profile = run(temp.path(), &["send", "missing-profile", "hello"]);
    assert_err_contains(&send_missing_profile, "unknown channel profile");

    assert_ok(&run(temp.path(), &["stop"]));
    assert_ok(&run(temp.path(), &["restart"]));
    assert_ok(&run(temp.path(), &["stop"]));
}

#[test]
fn doctor_reports_healthy_and_unhealthy_permutations() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(temp.path().join(".direclaw")).expect("create config dir");
    fs::write(
        temp.path().join(".direclaw/config.yaml"),
        format!(
            r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators: {{}}
channel_profiles: {{}}
monitoring: {{}}
channels:
  slack:
    enabled: false
"#,
            workspace = workspace.display()
        ),
    )
    .expect("write settings");

    let unhealthy = Command::new(env!("CARGO_BIN_EXE_direclaw"))
        .arg("doctor")
        .env("HOME", temp.path())
        .env("PATH", "")
        .output()
        .expect("run unhealthy doctor");
    assert_ok(&unhealthy);
    let unhealthy_out = stdout(&unhealthy);
    assert!(unhealthy_out.contains("summary=unhealthy"));
    assert!(unhealthy_out.contains("check:binary.anthropic=fail"));
    assert!(unhealthy_out.contains("check:binary.openai=fail"));

    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    for name in ["claude", "codex"] {
        let path = bin_dir.join(name);
        fs::write(&path, "#!/bin/sh\necho ok\n").expect("write shim");
    }
    let non_exec = Command::new(env!("CARGO_BIN_EXE_direclaw"))
        .arg("doctor")
        .env("HOME", temp.path())
        .env("PATH", bin_dir.display().to_string())
        .output()
        .expect("run non-executable doctor");
    assert_ok(&non_exec);
    let non_exec_out = stdout(&non_exec);
    assert!(non_exec_out.contains("summary=unhealthy"));
    assert!(non_exec_out.contains("check:binary.anthropic=fail"));
    assert!(non_exec_out.contains("check:binary.openai=fail"));

    for name in ["claude", "codex"] {
        let path = bin_dir.join(name);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).expect("chmod");
        }
    }
    let healthy = Command::new(env!("CARGO_BIN_EXE_direclaw"))
        .arg("doctor")
        .env("HOME", temp.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin_dir.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .expect("run healthy doctor");
    assert_ok(&healthy);
    let healthy_out = stdout(&healthy);
    assert!(healthy_out.contains("summary=healthy"));
    assert!(healthy_out.contains("checks_failed=0"));
}

#[test]
fn cli_output_contracts_include_structured_health_and_remediation() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);
    assert_ok(&run(temp.path(), &["setup"]));

    let status = run(temp.path(), &["status"]);
    assert_ok(&status);
    let status_contract = kv_lines(&status);
    assert!(status_contract.contains_key("ownership"));
    assert!(status_contract.contains_key("running"));
    assert!(status_contract.contains_key("pid"));

    let update = run(temp.path(), &["update", "apply"]);
    assert_err_contains(
        &update,
        "update apply is unsupported in this build to avoid unsafe in-place upgrades",
    );
    assert_err_contains(
        &update,
        "remediation: visit GitHub Releases, download the target archive, verify SHA256, and replace the binary manually",
    );

    let doctor = run(temp.path(), &["doctor"]);
    assert_ok(&doctor);
    let doctor_out = stdout(&doctor);
    for required in [
        "summary=",
        "checks_total=",
        "checks_failed=",
        "check:config.path=",
    ] {
        assert!(
            doctor_out.contains(required),
            "doctor output missing `{required}`:\n{doctor_out}"
        );
    }

    let bad_update = run(temp.path(), &["update", "bogus"]);
    assert_err_contains(&bad_update, "usage: update [check|apply]");
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
        &["orchestrator", "grant-shared-access", "alpha", "shared"],
    ));
    assert_ok(&run(
        temp.path(),
        &["orchestrator", "revoke-shared-access", "alpha", "shared"],
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
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let claude = bin_dir.join("claude");
    let codex = bin_dir.join("codex");
    fs::write(
        &claude,
        "#!/bin/sh\necho '[workflow_result]{\"result\":\"ok\"}[/workflow_result]'\n",
    )
    .expect("write claude mock");
    fs::write(
        &codex,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"result\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    )
    .expect("write codex mock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [&claude, &codex] {
            let mut perms = fs::metadata(path).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("chmod");
        }
    }

    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));
    assert_ok(&run(temp.path(), &["workflow", "list", "alpha"]));
    assert_ok(&run(temp.path(), &["workflow", "add", "alpha", "triage"]));
    assert_ok(&run(temp.path(), &["workflow", "show", "alpha", "triage"]));
    assert_ok(&run(
        temp.path(),
        &["orchestrator", "set-default-workflow", "alpha", "triage"],
    ));

    let run_output = run_with_env(
        temp.path(),
        &[
            "workflow",
            "run",
            "alpha",
            "triage",
            "--input",
            "ticket=123",
        ],
        &[
            (
                "DIRECLAW_PROVIDER_BIN_ANTHROPIC",
                claude.to_str().expect("utf8"),
            ),
            (
                "DIRECLAW_PROVIDER_BIN_OPENAI",
                codex.to_str().expect("utf8"),
            ),
        ],
    );
    assert_ok(&run_output);
    let run_id = run_id_from(&run_output);

    let status = run(temp.path(), &["workflow", "status", &run_id]);
    assert_ok(&status);
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(status_text.contains("state=succeeded"));
    assert!(status_text.contains("input_count=1"));
    assert!(status_text.contains("input_keys=ticket"));

    let progress = run(temp.path(), &["workflow", "progress", &run_id]);
    assert_ok(&progress);
    let progress_stdout = String::from_utf8_lossy(&progress.stdout);
    let progress_json_start = progress_stdout
        .find('{')
        .expect("progress output should include json object");
    let progress_json: serde_json::Value =
        serde_json::from_str(&progress_stdout[progress_json_start..]).expect("parse progress json");
    assert_eq!(progress_json["inputCount"], serde_json::Value::from(1));
    assert_eq!(progress_json["inputKeys"], serde_json::json!(["ticket"]));

    let run_record_path = temp
        .path()
        .join(".direclaw/workflows/runs")
        .join(format!("{run_id}.json"));
    let run_record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&run_record_path).expect("read run record"))
            .expect("parse run record");
    assert_eq!(
        run_record["inputs"]["ticket"],
        serde_json::Value::String("123".to_string())
    );

    assert_ok(&run(temp.path(), &["workflow", "cancel", &run_id]));
}

#[test]
fn workflow_run_denies_ungranted_agent_workspace_and_creates_no_workspace_dirs() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));
    let private_workspace = temp.path().join("workspace").join("alpha");
    let denied_workspace = temp.path().join("denied-agent-workspace");
    let orchestrators_path = temp.path().join(".direclaw/config-orchestrators.yaml");
    let mut orchestrators: BTreeMap<String, OrchestratorConfig> =
        serde_yaml::from_str(&fs::read_to_string(&orchestrators_path).expect("read orchestrators"))
            .expect("parse orchestrators");
    orchestrators
        .get_mut("alpha")
        .expect("alpha orchestrator")
        .agents
        .get_mut("default")
        .expect("default agent")
        .private_workspace = Some(denied_workspace.clone());
    fs::write(
        &orchestrators_path,
        serde_yaml::to_string(&orchestrators).expect("serialize orchestrators"),
    )
    .expect("write orchestrators");

    assert!(!private_workspace.join("workflows/runs").exists());
    assert!(!denied_workspace.exists());

    let run_output = run(temp.path(), &["workflow", "run", "alpha", "default"]);
    assert_err_contains(&run_output, "workspace access denied");

    assert!(!private_workspace.join("workflows/runs").exists());
    assert!(!denied_workspace.exists());
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
