use direclaw::config::{
    AgentConfig, ConfigProviderKind, OrchestratorConfig, OutputKey, PathTemplate, StepLimitsConfig,
    WorkflowConfig, WorkflowInputs, WorkflowLimitsConfig, WorkflowOrchestrationConfig,
    WorkflowStepConfig, WorkflowStepPromptType, WorkflowStepType, WorkflowStepWorkspaceMode,
};
use std::collections::{BTreeMap, BTreeSet};
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

fn out_keys(keys: &[&str]) -> Vec<OutputKey> {
    keys.iter()
        .map(|key| OutputKey::parse(key).expect("valid output key"))
        .collect()
}

fn out_files(entries: &[(&str, &str)]) -> BTreeMap<OutputKey, PathTemplate> {
    entries
        .iter()
        .map(|(key, path)| {
            (
                OutputKey::parse_output_file_key(key).expect("valid output file key"),
                PathTemplate::parse(path).expect("valid path template"),
            )
        })
        .collect()
}

fn write_settings(home: &Path, include_shared_workspace: bool) {
    let workspace = home.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("create config dir");
    let shared_workspace = home.join("shared-workspace");
    fs::create_dir_all(&shared_workspace).expect("create shared workspace");

    let shared = if include_shared_workspace {
        format!(
            "  shared:\n    path: {}\n    description: shared workspace for cli tests",
            shared_workspace.display()
        )
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
  heartbeat_interval: 0
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

fn assert_compact_run_id(run_id: &str) {
    let mut segments = run_id.split('-');
    assert_eq!(segments.next(), Some("run"));
    let timestamp = segments.next().expect("base36 timestamp segment");
    assert!(
        !timestamp.is_empty(),
        "run id timestamp segment must not be empty"
    );
    assert!(
        timestamp
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch.is_ascii_lowercase()),
        "run id timestamp segment must be lowercase base36: {run_id}"
    );
    let suffix = segments.next().expect("suffix segment");
    assert_eq!(suffix.len(), 4, "run id suffix length mismatch: {run_id}");
    assert!(
        suffix
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch.is_ascii_lowercase()),
        "run id suffix must be lowercase base36: {run_id}"
    );
    assert!(
        segments.next().is_none(),
        "run id must contain exactly three '-' separated segments: {run_id}"
    );
}

fn workflow_run_ids(home: &Path) -> BTreeSet<String> {
    let workspace_root = home.join("workspace");
    if !workspace_root.exists() {
        return BTreeSet::new();
    }

    let mut ids = BTreeSet::new();
    for workspace in fs::read_dir(&workspace_root).expect("read workspace root") {
        let runs_dir = workspace
            .expect("workspace entry")
            .path()
            .join("workflows/runs");
        if !runs_dir.exists() {
            continue;
        }
        for run in fs::read_dir(&runs_dir).expect("read runs dir") {
            let path = run.expect("run entry").path();
            if path.extension().and_then(|v| v.to_str()) != Some("json") {
                continue;
            }
            if let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) {
                ids.insert(id.to_string());
            }
        }
    }
    ids
}

#[test]
fn daemon_start_status_stop_command_surface_works() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["setup"]));
    assert_ok(&run(temp.path(), &["start", "--detach"]));

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

    assert_ok(&run(temp.path(), &["stop"]));
}

#[test]
fn daemon_restart_command_surface_works() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);
    assert_ok(&run(temp.path(), &["setup"]));
    assert_ok(&run(temp.path(), &["restart", "--detach"]));
    assert_ok(&run(temp.path(), &["stop"]));
}

#[test]
fn daemon_auxiliary_command_surface_works() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["setup"]));
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
}

#[test]
fn start_missing_global_config_suggests_setup() {
    let temp = tempdir().expect("tempdir");

    let started = run(temp.path(), &["start", "--detach"]);
    assert_err_contains(&started, "failed to read file");
    assert_err_contains(&started, "direclaw setup");
}

#[test]
fn selector_function_id_aliases_work_on_cli_surface() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["setup"]));
    assert_ok(&run(temp.path(), &["daemon.start"]));

    let dashed = run(temp.path(), &["channel-profile", "list"]);
    let dotted = run(temp.path(), &["channel_profile.list"]);
    assert_ok(&dashed);
    assert_ok(&dotted);
    assert!(stdout(&dashed).contains("local-default"));
    assert!(stdout(&dotted).contains("local-default"));

    assert_ok(&run(temp.path(), &["daemon.stop"]));
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
fn orchestrator_commands_work() {
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
}

#[test]
fn orchestrator_agent_commands_work() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);
    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));

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
        "#!/bin/sh\necho '[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"ok\"}[/workflow_result]'\n",
    )
    .expect("write claude mock");
    fs::write(
        &codex,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
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

    let orchestrator_path = temp.path().join("workspace/alpha/orchestrator.yaml");
    let mut orchestrator: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&orchestrator_path).expect("read orchestrator"))
            .expect("parse orchestrator");
    let triage = orchestrator
        .workflows
        .iter_mut()
        .find(|workflow| workflow.id == "triage")
        .expect("triage workflow");
    let first_step = triage.steps.first_mut().expect("triage step");
    first_step.prompt_type = WorkflowStepPromptType::WorkflowResultEnvelope;
    fs::write(
        &orchestrator_path,
        serde_yaml::to_string(&orchestrator).expect("serialize orchestrator"),
    )
    .expect("write orchestrator");

    let show = run(temp.path(), &["workflow", "show", "alpha", "triage"]);
    assert_ok(&show);
    let show_text = stdout(&show);
    assert!(show_text.contains(".prompt.md"));
    assert!(show_text.contains("prompt_type: workflow_result_envelope"));
    assert!(show_text.contains("outputs:"));
    assert!(show_text.contains("output_files:"));
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
    assert_compact_run_id(&run_id);

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
        .join("workspace/alpha/workflows/runs")
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
fn fresh_setup_default_workflow_runs_successfully() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);
    assert_ok(&run(temp.path(), &["setup"]));

    let orchestrator_path = temp.path().join("workspace/main/orchestrator.yaml");
    let mut orchestrator: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&orchestrator_path).expect("read orchestrator"))
            .expect("parse orchestrator");
    let default_workflow = orchestrator
        .workflows
        .iter_mut()
        .find(|workflow| workflow.id == "default")
        .expect("default workflow");
    let first_step = default_workflow.steps.first_mut().expect("default step");
    first_step.prompt_type = WorkflowStepPromptType::WorkflowResultEnvelope;
    fs::write(
        &orchestrator_path,
        serde_yaml::to_string(&orchestrator).expect("serialize orchestrator"),
    )
    .expect("write orchestrator");

    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let claude = bin_dir.join("claude");
    fs::write(
        &claude,
        "#!/bin/sh\necho '[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"ok\"}[/workflow_result]'\n",
    )
    .expect("write claude mock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&claude).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&claude, perms).expect("chmod");
    }

    let run_output = run_with_env(
        temp.path(),
        &["workflow", "run", "main", "default"],
        &[(
            "DIRECLAW_PROVIDER_BIN_ANTHROPIC",
            claude.to_str().expect("utf8"),
        )],
    );
    assert_ok(&run_output);
    let run_id = run_id_from(&run_output);
    assert_compact_run_id(&run_id);
    let status = run(temp.path(), &["workflow", "status", &run_id]);
    assert_ok(&status);
    assert!(stdout(&status).contains("state=succeeded"));
}

#[test]
fn workflow_step_workspace_mode_controls_provider_working_directory() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);
    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));

    let orchestrator_path = temp.path().join("workspace/alpha/orchestrator.yaml");
    let mut orchestrator: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&orchestrator_path).expect("read orchestrator"))
            .expect("parse orchestrator");
    orchestrator.default_workflow = "cwd_modes".to_string();
    orchestrator.selector_agent = "default".to_string();
    orchestrator.agents.insert(
        "worker".to_string(),
        AgentConfig {
            provider: ConfigProviderKind::OpenAi,
            model: "gpt-5.2".to_string(),
            private_workspace: None,
            can_orchestrate_workflows: false,
            shared_access: Vec::new(),
        },
    );
    orchestrator.workflows = vec![WorkflowConfig {
        id: "cwd_modes".to_string(),
        version: 1,
        description: "validate workspace mode routing".to_string(),
        tags: vec!["workspace".parse().expect("tag")],
        inputs: WorkflowInputs::default(),
        limits: None,
        steps: vec![
            WorkflowStepConfig {
                id: "s1".to_string(),
                step_type: WorkflowStepType::AgentTask,
                agent: "worker".to_string(),
                prompt: "s1".to_string(),
                prompt_type: WorkflowStepPromptType::WorkflowResultEnvelope,
                workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                next: Some("s2".to_string()),
                on_approve: None,
                on_reject: None,
                outputs: out_keys(&["summary", "artifact"]),
                output_files: out_files(&[
                    (
                        "summary",
                        "outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt",
                    ),
                    (
                        "artifact",
                        "outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt",
                    ),
                ]),
                final_output_priority: out_keys(&["artifact", "summary"]),
                limits: None,
            },
            WorkflowStepConfig {
                id: "s2".to_string(),
                step_type: WorkflowStepType::AgentTask,
                agent: "worker".to_string(),
                prompt: "s2".to_string(),
                prompt_type: WorkflowStepPromptType::WorkflowResultEnvelope,
                workspace_mode: WorkflowStepWorkspaceMode::RunWorkspace,
                next: Some("s3".to_string()),
                on_approve: None,
                on_reject: None,
                outputs: out_keys(&["summary", "artifact"]),
                output_files: out_files(&[
                    (
                        "summary",
                        "outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt",
                    ),
                    (
                        "artifact",
                        "outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt",
                    ),
                ]),
                final_output_priority: out_keys(&["artifact", "summary"]),
                limits: None,
            },
            WorkflowStepConfig {
                id: "s3".to_string(),
                step_type: WorkflowStepType::AgentTask,
                agent: "worker".to_string(),
                prompt: "s3".to_string(),
                prompt_type: WorkflowStepPromptType::WorkflowResultEnvelope,
                workspace_mode: WorkflowStepWorkspaceMode::AgentWorkspace,
                next: None,
                on_approve: None,
                on_reject: None,
                outputs: out_keys(&["summary", "artifact"]),
                output_files: out_files(&[
                    (
                        "summary",
                        "outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt",
                    ),
                    (
                        "artifact",
                        "outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt",
                    ),
                ]),
                final_output_priority: out_keys(&["artifact", "summary"]),
                limits: None,
            },
        ],
    }];
    fs::write(
        &orchestrator_path,
        serde_yaml::to_string(&orchestrator).expect("serialize orchestrator"),
    )
    .expect("write orchestrator");

    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let codex = bin_dir.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    )
    .expect("write codex mock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&codex).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&codex, perms).expect("chmod");
    }

    let run_output = run_with_env(
        temp.path(),
        &["workflow", "run", "alpha", "cwd_modes"],
        &[(
            "DIRECLAW_PROVIDER_BIN_OPENAI",
            codex.to_str().expect("utf8"),
        )],
    );
    assert_ok(&run_output);
    let run_id = run_id_from(&run_output);

    let invoc = |step: &str| -> serde_json::Value {
        let path = temp.path().join(format!(
            "workspace/alpha/workflows/runs/{run_id}/steps/{step}/attempts/1/provider_invocation.json"
        ));
        serde_json::from_str(&fs::read_to_string(path).expect("read invocation"))
            .expect("parse invocation")
    };
    let s1 = invoc("s1");
    let s2 = invoc("s2");
    let s3 = invoc("s3");
    let s1_cwd = s1["workingDirectory"].as_str().expect("s1 cwd");
    let s2_cwd = s2["workingDirectory"].as_str().expect("s2 cwd");
    let s3_cwd = s3["workingDirectory"].as_str().expect("s3 cwd");
    assert_eq!(
        s1_cwd,
        temp.path().join("workspace/alpha").display().to_string()
    );
    assert_eq!(
        s2_cwd,
        temp.path()
            .join(format!("workspace/alpha/workflows/runs/{run_id}/workspace"))
            .display()
            .to_string()
    );
    assert_eq!(
        s3_cwd,
        temp.path()
            .join("workspace/alpha/agents/worker")
            .display()
            .to_string()
    );
}

#[test]
fn workflow_runtime_consumes_tui_style_fields_end_to_end() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));

    let orchestrator_path = temp.path().join("workspace/alpha/orchestrator.yaml");
    let mut orchestrator: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&orchestrator_path).expect("read orchestrator"))
            .expect("parse orchestrator");
    orchestrator.default_workflow = "triage_roundtrip".to_string();
    orchestrator.selector_agent = "default".to_string();
    orchestrator.workflow_orchestration = Some(WorkflowOrchestrationConfig {
        max_total_iterations: Some(8),
        default_run_timeout_seconds: Some(25),
        default_step_timeout_seconds: Some(5),
        max_step_timeout_seconds: Some(5),
    });
    orchestrator.agents.insert(
        "default".to_string(),
        AgentConfig {
            provider: ConfigProviderKind::Anthropic,
            model: "sonnet".to_string(),
            private_workspace: None,
            can_orchestrate_workflows: true,
            shared_access: Vec::new(),
        },
    );
    orchestrator.agents.insert(
        "worker".to_string(),
        AgentConfig {
            provider: ConfigProviderKind::OpenAi,
            model: "gpt-5.2".to_string(),
            private_workspace: None,
            can_orchestrate_workflows: false,
            shared_access: Vec::new(),
        },
    );
    orchestrator.workflows = vec![WorkflowConfig {
        id: "triage_roundtrip".to_string(),
        version: 1,
        description: "roundtrip typed workflow fields across runtime".to_string(),
        tags: vec!["roundtrip".parse().expect("tag")],
        inputs: WorkflowInputs::parse_keys(["ticket", "priority"]).expect("valid workflow inputs"),
        limits: Some(WorkflowLimitsConfig {
            max_total_iterations: Some(7),
            run_timeout_seconds: Some(20),
        }),
        steps: vec![
            WorkflowStepConfig {
                id: "plan".to_string(),
                step_type: WorkflowStepType::AgentTask,
                agent: "worker".to_string(),
                prompt: "plan ticket={{inputs.ticket}} priority={{inputs.priority}}".to_string(),
                prompt_type: WorkflowStepPromptType::WorkflowResultEnvelope,
                workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                next: Some("review".to_string()),
                on_approve: None,
                on_reject: None,
                outputs: out_keys(&["plan", "summary"]),
                output_files: out_files(&[
                    (
                        "plan",
                        "reports/{{workflow.run_id}}/plan-{{workflow.attempt}}.md",
                    ),
                    (
                        "summary",
                        "reports/{{workflow.run_id}}/summary-{{workflow.attempt}}.txt",
                    ),
                ]),
                final_output_priority: out_keys(&["summary"]),
                limits: Some(StepLimitsConfig {
                    max_retries: Some(1),
                }),
            },
            WorkflowStepConfig {
                id: "review".to_string(),
                step_type: WorkflowStepType::AgentReview,
                agent: "worker".to_string(),
                prompt: "review run {{workflow.run_id}}".to_string(),
                prompt_type: WorkflowStepPromptType::WorkflowResultEnvelope,
                workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                next: None,
                on_approve: Some("finalize".to_string()),
                on_reject: Some("plan".to_string()),
                outputs: out_keys(&["decision", "summary", "feedback"]),
                output_files: out_files(&[
                    (
                        "decision",
                        "reports/{{workflow.run_id}}/decision-{{workflow.attempt}}.txt",
                    ),
                    (
                        "summary",
                        "reports/{{workflow.run_id}}/review-summary-{{workflow.attempt}}.txt",
                    ),
                    (
                        "feedback",
                        "reports/{{workflow.run_id}}/review-feedback-{{workflow.attempt}}.txt",
                    ),
                ]),
                final_output_priority: out_keys(&["summary"]),
                limits: None,
            },
            WorkflowStepConfig {
                id: "finalize".to_string(),
                step_type: WorkflowStepType::AgentTask,
                agent: "worker".to_string(),
                prompt: "finalize run {{workflow.run_id}}".to_string(),
                prompt_type: WorkflowStepPromptType::WorkflowResultEnvelope,
                workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                next: None,
                on_approve: None,
                on_reject: None,
                outputs: out_keys(&["summary", "result"]),
                output_files: out_files(&[
                    ("summary", "reports/{{workflow.run_id}}/final-summary.txt"),
                    ("result", "reports/{{workflow.run_id}}/result.json"),
                ]),
                final_output_priority: out_keys(&["summary", "result"]),
                limits: None,
            },
        ],
    }];
    fs::write(
        &orchestrator_path,
        serde_yaml::to_string(&orchestrator).expect("serialize orchestrator"),
    )
    .expect("write orchestrator");

    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let codex = bin_dir.join("codex");
    fs::write(
        &codex,
        r#"#!/bin/sh
set -eu
args="$*"
marker="$PWD/.first_plan_attempt_failed"
if printf "%s" "$args" | grep -q "/steps/plan/" && [ ! -f "$marker" ]; then
  touch "$marker"
  echo '{not-json}'
  exit 0
fi
if printf "%s" "$args" | grep -q "/steps/review/"; then
  echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"decision\":\"approve\",\"summary\":\"looks good\",\"feedback\":\"none\"}[/workflow_result]"}}'
elif printf "%s" "$args" | grep -q "/steps/finalize/"; then
  echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"summary\":\"done\",\"result\":{\"status\":\"done\"}}[/workflow_result]"}}'
else
  echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"plan\":\"Use checks\",\"summary\":\"roundtrip-ok\"}[/workflow_result]"}}'
fi
"#,
    )
    .expect("write codex mock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&codex).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&codex, perms).expect("chmod");
    }

    let list = run(temp.path(), &["workflow", "list", "alpha"]);
    assert_ok(&list);
    assert!(stdout(&list).contains("triage_roundtrip"));
    let show = run(
        temp.path(),
        &["workflow", "show", "alpha", "triage_roundtrip"],
    );
    assert_ok(&show);
    let show_text = stdout(&show);
    assert!(show_text.contains("inputs:"));
    assert!(show_text.contains("output_files:"));
    assert!(show_text.contains("max_retries: 1"));

    let run_output = run_with_env(
        temp.path(),
        &[
            "workflow",
            "run",
            "alpha",
            "triage_roundtrip",
            "--input",
            "ticket=123",
            "--input",
            "priority=high",
        ],
        &[(
            "DIRECLAW_PROVIDER_BIN_OPENAI",
            codex.to_str().expect("utf8"),
        )],
    );
    assert_ok(&run_output);
    let run_id = run_id_from(&run_output);

    let status = run(temp.path(), &["workflow", "status", &run_id]);
    assert_ok(&status);
    let status_text = stdout(&status);
    assert!(status_text.contains("state=succeeded"));
    assert!(status_text.contains("input_count=2"));
    assert!(status_text.contains("input_keys=priority,ticket"));

    let progress = run(temp.path(), &["workflow", "progress", &run_id]);
    assert_ok(&progress);
    let progress_stdout = stdout(&progress);
    let progress_json_start = progress_stdout
        .find('{')
        .expect("progress output should include json object");
    let progress_json: serde_json::Value =
        serde_json::from_str(&progress_stdout[progress_json_start..]).expect("progress json");
    assert_eq!(
        progress_json["state"],
        serde_json::Value::String("succeeded".to_string())
    );

    let run_root = temp
        .path()
        .join("workspace/alpha/workflows/runs")
        .join(&run_id);
    let plan_attempt_1: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(run_root.join("steps/plan/attempts/1/result.json"))
            .expect("plan attempt 1"),
    )
    .expect("parse attempt 1");
    let plan_attempt_2: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(run_root.join("steps/plan/attempts/2/result.json"))
            .expect("plan attempt 2"),
    )
    .expect("parse attempt 2");
    assert_eq!(
        plan_attempt_1["state"],
        serde_json::Value::String("failed_retryable".to_string())
    );
    assert_eq!(
        plan_attempt_2["state"],
        serde_json::Value::String("succeeded".to_string())
    );

    let plan_output = run_root.join(format!("steps/plan/attempts/2/reports/{run_id}/plan-2.md"));
    let summary_output = run_root.join(format!(
        "steps/plan/attempts/2/reports/{run_id}/summary-2.txt"
    ));
    let result_output = run_root.join(format!(
        "steps/finalize/attempts/1/reports/{run_id}/result.json"
    ));
    assert_eq!(
        fs::read_to_string(plan_output).expect("plan output"),
        "Use checks"
    );
    assert_eq!(
        fs::read_to_string(summary_output).expect("summary output"),
        "roundtrip-ok"
    );
    assert!(fs::read_to_string(result_output)
        .expect("result output")
        .contains("\"status\": \"done\""));

    assert_ok(&run(temp.path(), &["workflow", "cancel", &run_id]));
}

#[test]
fn workflow_inputs_persist_and_reload_after_setup_style_edit() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);
    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));

    let orchestrator_path = temp.path().join("workspace/alpha/orchestrator.yaml");
    let mut orchestrator: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&orchestrator_path).expect("read orchestrator"))
            .expect("parse orchestrator");
    let workflow_id = orchestrator.default_workflow.clone();
    let workflow = orchestrator
        .workflows
        .iter_mut()
        .find(|workflow| workflow.id == workflow_id)
        .expect("default workflow");
    workflow.inputs = WorkflowInputs::parse_keys(["ticket", "priority", "ticket"])
        .expect("valid workflow inputs");
    fs::write(
        &orchestrator_path,
        serde_yaml::to_string(&orchestrator).expect("serialize orchestrator"),
    )
    .expect("write orchestrator");

    let reloaded: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&orchestrator_path).expect("read orchestrator"))
            .expect("parse orchestrator");
    let reloaded_workflow = reloaded
        .workflows
        .iter()
        .find(|workflow| workflow.id == workflow_id)
        .expect("reloaded workflow");
    let keys = reloaded_workflow
        .inputs
        .as_slice()
        .iter()
        .map(|key| key.as_str().to_string())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["ticket".to_string(), "priority".to_string()]);

    let show = run(temp.path(), &["workflow", "show", "alpha", &workflow_id]);
    assert_ok(&show);
    let show_text = stdout(&show);
    assert!(show_text.contains("inputs:"));
    assert!(show_text.contains("- ticket"));
    assert!(show_text.contains("- priority"));
}

#[test]
fn workflow_run_enforces_step_timeout_from_cli_config() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);
    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));

    let orchestrator_path = temp.path().join("workspace/alpha/orchestrator.yaml");
    let mut orchestrator: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&orchestrator_path).expect("read orchestrator"))
            .expect("parse orchestrator");
    orchestrator.default_workflow = "timeout_roundtrip".to_string();
    orchestrator.selector_agent = "default".to_string();
    orchestrator.workflow_orchestration = Some(WorkflowOrchestrationConfig {
        max_total_iterations: Some(4),
        default_run_timeout_seconds: Some(30),
        default_step_timeout_seconds: Some(5),
        max_step_timeout_seconds: Some(0),
    });
    orchestrator.agents.insert(
        "default".to_string(),
        AgentConfig {
            provider: ConfigProviderKind::OpenAi,
            model: "gpt-5.2".to_string(),
            private_workspace: None,
            can_orchestrate_workflows: true,
            shared_access: Vec::new(),
        },
    );
    orchestrator.agents.insert(
        "worker".to_string(),
        AgentConfig {
            provider: ConfigProviderKind::OpenAi,
            model: "gpt-5.2".to_string(),
            private_workspace: None,
            can_orchestrate_workflows: false,
            shared_access: Vec::new(),
        },
    );
    orchestrator.workflows = vec![WorkflowConfig {
        id: "timeout_roundtrip".to_string(),
        version: 1,
        description: "exercise per-step timeout limits".to_string(),
        tags: vec!["timeout".parse().expect("tag")],
        inputs: WorkflowInputs::default(),
        limits: None,
        steps: vec![WorkflowStepConfig {
            id: "slow".to_string(),
            step_type: WorkflowStepType::AgentTask,
            agent: "worker".to_string(),
            prompt: "slow".to_string(),
            prompt_type: WorkflowStepPromptType::WorkflowResultEnvelope,
            workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
            next: None,
            on_approve: None,
            on_reject: None,
            outputs: out_keys(&["summary", "result"]),
            output_files: out_files(&[
                (
                    "summary",
                    "reports/{{workflow.run_id}}/slow-summary-{{workflow.attempt}}.txt",
                ),
                (
                    "result",
                    "reports/{{workflow.run_id}}/slow-{{workflow.attempt}}.txt",
                ),
            ]),
            final_output_priority: out_keys(&["summary", "result"]),
            limits: None,
        }],
    }];
    fs::write(
        &orchestrator_path,
        serde_yaml::to_string(&orchestrator).expect("serialize orchestrator"),
    )
    .expect("write orchestrator");

    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let codex = bin_dir.join("codex");
    fs::write(
        &codex,
        r#"#!/bin/sh
set -eu
while :; do :; done
echo '{"type":"item.completed","item":{"type":"agent_message","text":"[workflow_result]{\"summary\":\"slow-ok\",\"result\":\"slow-ok\"}[/workflow_result]"}}'
"#,
    )
    .expect("write codex mock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&codex).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&codex, perms).expect("chmod");
    }

    let before_ids = workflow_run_ids(temp.path());
    let step_timeout_run = run_with_env(
        temp.path(),
        &["workflow", "run", "alpha", "timeout_roundtrip"],
        &[(
            "DIRECLAW_PROVIDER_BIN_OPENAI",
            codex.to_str().expect("utf8"),
        )],
    );
    assert_err_contains(&step_timeout_run, "workflow step timed out after 0s");

    let after_step_timeout_ids = workflow_run_ids(temp.path());
    assert_eq!(after_step_timeout_ids.len(), before_ids.len() + 1);
    let first_new_run_id = after_step_timeout_ids
        .difference(&before_ids)
        .next()
        .expect("new run id after step-timeout run")
        .to_string();
    let first_run_record: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            temp.path()
                .join("workspace/alpha/workflows/runs")
                .join(format!("{first_new_run_id}.json")),
        )
        .expect("read first run record"),
    )
    .expect("parse first run record");
    assert_eq!(
        first_run_record["state"],
        serde_json::Value::String("failed".to_string())
    );
    assert!(first_run_record["terminalReason"]
        .as_str()
        .unwrap_or_default()
        .contains("workflow step timed out after 0s"));
}

#[test]
fn workflow_run_denies_ungranted_agent_workspace_and_creates_no_workspace_dirs() {
    let temp = tempdir().expect("tempdir");
    write_settings(temp.path(), true);

    assert_ok(&run(temp.path(), &["orchestrator", "add", "alpha"]));
    let private_workspace = temp.path().join("workspace").join("alpha");
    let denied_workspace = temp.path().join("denied-agent-workspace");
    let orchestrator_path = temp.path().join("workspace/alpha/orchestrator.yaml");
    let mut orchestrator: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&orchestrator_path).expect("read orchestrator"))
            .expect("parse orchestrator");
    orchestrator
        .agents
        .get_mut("default")
        .expect("default agent")
        .private_workspace = Some(denied_workspace.clone());
    fs::write(
        &orchestrator_path,
        serde_yaml::to_string(&orchestrator).expect("serialize orchestrator"),
    )
    .expect("write orchestrator");

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
