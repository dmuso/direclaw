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
fn config_paths_module_exposes_default_path_helpers() {
    assert_eq!(direclaw::config::paths::GLOBAL_STATE_DIR, ".direclaw");
    assert_eq!(
        direclaw::config::paths::GLOBAL_SETTINGS_FILE_NAME,
        "config.yaml"
    );
    assert_eq!(
        direclaw::config::paths::GLOBAL_ORCHESTRATORS_FILE_NAME,
        "config-orchestrators.yaml"
    );
}

#[test]
fn config_settings_module_exposes_validation_options() {
    let options = direclaw::config::settings::ValidationOptions::default();
    assert!(options.require_shared_paths_exist);
}

#[test]
fn config_orchestrator_file_module_exposes_provider_kind_parser() {
    let provider = direclaw::config::orchestrator_file::ConfigProviderKind::parse("openai")
        .expect("parse provider");
    assert_eq!(
        provider,
        direclaw::config::orchestrator_file::ConfigProviderKind::OpenAi
    );
}

#[test]
fn config_load_module_exposes_loader_functions() {
    let _settings_loader: fn() -> Result<Settings, direclaw::config::ConfigError> =
        direclaw::config::load::load_global_settings;
    let _orchestrator_loader: fn(
        &Settings,
        &str,
    ) -> Result<OrchestratorConfig, direclaw::config::ConfigError> =
        direclaw::config::load::load_orchestrator_config;
}

#[test]
fn config_validate_module_exposes_validation_entry_points() {
    let _settings_validate: fn(
        &Settings,
        ValidationOptions,
    ) -> Result<(), direclaw::config::ConfigError> = direclaw::config::validate::validate_settings;
    let _orchestrator_validate: fn(
        &OrchestratorConfig,
        &Settings,
        &str,
    ) -> Result<(), direclaw::config::ConfigError> =
        direclaw::config::validate::validate_orchestrator_config;
}

#[test]
fn app_command_catalog_module_exposes_v1_functions() {
    let has_start = direclaw::app::command_catalog::V1_FUNCTIONS
        .iter()
        .any(|def| def.function_id == direclaw::app::command_catalog::function_ids::DAEMON_START);
    assert!(has_start);
}

#[test]
fn app_command_dispatch_module_exposes_invocation_planner() {
    let _planner: fn(
        &str,
        &serde_json::Map<String, serde_json::Value>,
    )
        -> Result<direclaw::app::command_dispatch::FunctionExecutionPlan, String> =
        direclaw::app::command_dispatch::plan_function_invocation;
}

#[test]
fn app_cli_module_exposes_cli_verb_and_help_surface() {
    let verb = direclaw::app::cli::parse_cli_verb("start");
    assert_eq!(verb, direclaw::app::cli::CliVerb::Start);

    let lines = direclaw::app::cli::cli_help_lines();
    assert!(lines.iter().any(|line| line.contains("start")));
}

#[test]
fn app_provider_command_handler_module_exposes_provider_and_model_commands() {
    let _provider_cmd: fn(&[String]) -> Result<String, String> =
        direclaw::app::command_handlers::provider::cmd_provider;
    let _model_cmd: fn(&[String]) -> Result<String, String> =
        direclaw::app::command_handlers::provider::cmd_model;
}

#[test]
fn app_doctor_and_attach_command_handler_modules_expose_commands() {
    let _doctor_cmd: fn() -> Result<String, String> =
        direclaw::app::command_handlers::doctor::cmd_doctor;
    let _attach_cmd: fn() -> Result<String, String> =
        direclaw::app::command_handlers::attach::cmd_attach;
}

#[test]
fn app_daemon_command_handler_module_exposes_commands() {
    let _start_cmd: fn() -> Result<String, String> =
        direclaw::app::command_handlers::daemon::cmd_start;
    let _stop_cmd: fn() -> Result<String, String> =
        direclaw::app::command_handlers::daemon::cmd_stop;
    let _restart_cmd: fn() -> Result<String, String> =
        direclaw::app::command_handlers::daemon::cmd_restart;
    let _status_cmd: fn() -> Result<String, String> =
        direclaw::app::command_handlers::daemon::cmd_status;
    let _logs_cmd: fn() -> Result<String, String> =
        direclaw::app::command_handlers::daemon::cmd_logs;
    let _supervisor_cmd: fn(&[String]) -> Result<String, String> =
        direclaw::app::command_handlers::daemon::cmd_supervisor;
}

#[test]
fn app_channel_profile_command_handler_module_exposes_command() {
    let _channel_profile_cmd: fn(&[String]) -> Result<String, String> =
        direclaw::app::command_handlers::channel_profiles::cmd_channel_profile;
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
