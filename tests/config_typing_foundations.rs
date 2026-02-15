use direclaw::config::{
    ChannelKind, ConfigProviderKind, OrchestratorConfig, Settings, ValidationOptions,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
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

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn typed_fields_module_exposes_workflow_key_normalizer() {
    let normalized = direclaw::config::typed_fields::normalize_workflow_input_key("  ticket_id  ")
        .expect("normalize key");
    assert_eq!(normalized, "ticket_id");
}

#[test]
fn spec_example_settings_and_orchestrators_load_with_typed_fields() {
    let mut settings_by_orchestrator = BTreeMap::new();
    for settings_file in [
        "docs/build/spec/examples/settings/minimal.settings.yaml",
        "docs/build/spec/examples/settings/full.settings.yaml",
    ] {
        let path = project_root().join(settings_file);
        let settings = Settings::from_path(&path).expect("parse settings example");
        settings
            .validate(ValidationOptions {
                require_shared_paths_exist: false,
            })
            .expect("settings validate");
        for orchestrator_id in settings.orchestrators.keys() {
            settings_by_orchestrator.insert(orchestrator_id.clone(), settings.clone());
        }
        for profile in settings.channel_profiles.values() {
            match profile.channel {
                ChannelKind::Local
                | ChannelKind::Slack
                | ChannelKind::Discord
                | ChannelKind::Telegram
                | ChannelKind::Whatsapp => {}
            }
        }
    }

    for orchestrator_file in [
        "docs/build/spec/examples/orchestrators/minimal.orchestrator.yaml",
        "docs/build/spec/examples/orchestrators/engineering.orchestrator.yaml",
        "docs/build/spec/examples/orchestrators/product.orchestrator.yaml",
    ] {
        let path = project_root().join(orchestrator_file);
        let orchestrator = OrchestratorConfig::from_path(&path).expect("parse orchestrator");
        let settings = settings_by_orchestrator
            .get(&orchestrator.id)
            .unwrap_or_else(|| {
                panic!(
                    "missing settings fixture for orchestrator `{}`",
                    orchestrator.id
                )
            });
        for agent in orchestrator.agents.values() {
            match agent.provider {
                ConfigProviderKind::Anthropic | ConfigProviderKind::OpenAi => {}
            }
        }
        orchestrator
            .validate(settings, &orchestrator.id)
            .expect("orchestrator validate");
    }
}

#[test]
fn commands_reject_invalid_id_values() {
    let temp = tempdir().expect("tempdir");
    assert_ok(&run(temp.path(), &["setup"]));

    let invalid_orchestrator = run(temp.path(), &["orchestrator", "add", "bad id"]);
    assert_err_contains(
        &invalid_orchestrator,
        "orchestrator id must use only ASCII letters, digits, '-' or '_'",
    );

    let invalid_workflow = run(temp.path(), &["workflow", "add", "main", "bad id"]);
    assert_err_contains(
        &invalid_workflow,
        "workflow id must use only ASCII letters, digits, '-' or '_'",
    );

    let invalid_agent = run(
        temp.path(),
        &["orchestrator-agent", "add", "main", "bad id"],
    );
    assert_err_contains(
        &invalid_agent,
        "agent id must use only ASCII letters, digits, '-' or '_'",
    );
}

#[test]
fn setup_fails_fast_for_invalid_existing_orchestrator_id() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    let workspace = home.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("create config dir");
    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  bad id:
    shared_access: []
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
            workspace.display()
        ),
    )
    .expect("write config");

    let setup = run(home, &["setup"]);
    assert_err_contains(
        &setup,
        "orchestrator id must use only ASCII letters, digits, '-' or '_'",
    );
}

#[test]
fn commands_surface_invalid_step_id_from_orchestrator_validation() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    let workspaces = home.join("workspaces");
    let orchestrator_workspace = workspaces.join("main");
    fs::create_dir_all(&orchestrator_workspace).expect("create orchestrator workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("create config dir");

    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    shared_access: []
channel_profiles:
  local:
    channel: local
    orchestrator_id: main
monitoring: {{}}
channels: {{}}
"#,
            workspaces.display()
        ),
    )
    .expect("write config");

    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: planner
default_workflow: deliver
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  planner:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
workflows:
  - id: deliver
    version: 1
    inputs: []
    steps:
      - id: bad id
        type: agent_task
        agent: planner
        prompt: Draft a response.
        outputs: [summary]
        output_files:
          summary: outputs/summary.txt
"#,
    )
    .expect("write orchestrator config");

    let output = run(home, &["workflow", "list", "main"]);
    assert_err_contains(
        &output,
        "step id must use only ASCII letters, digits, '-' or '_'",
    );
}
