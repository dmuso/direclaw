use direclaw::config::{OrchestratorConfig, Settings, SettingsOrchestrator};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::tempdir;

fn run_setup(home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_direclaw"))
        .arg("setup")
        .env("HOME", home)
        .output()
        .expect("run setup")
}

fn run_setup_with_script_keys(home: &Path, keys: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_direclaw"))
        .arg("setup")
        .env("HOME", home)
        .env("DIRECLAW_SETUP_SCRIPT_KEYS", keys)
        .output()
        .expect("run setup with scripted keys")
}

fn load_settings(home: &Path) -> Settings {
    Settings::from_path(&home.join(".direclaw/config.yaml")).expect("load settings")
}

fn orchestrator_path(settings: &Settings, orchestrator_id: &str) -> PathBuf {
    settings
        .resolve_private_workspace(orchestrator_id)
        .expect("resolve workspace")
        .join("orchestrator.yaml")
}

fn load_orchestrator(settings: &Settings, orchestrator_id: &str) -> OrchestratorConfig {
    serde_yaml::from_str(
        &fs::read_to_string(orchestrator_path(settings, orchestrator_id))
            .expect("read orchestrator"),
    )
    .expect("parse orchestrator")
}

fn save_state(home: &Path, settings: &Settings, configs: &[(&str, &OrchestratorConfig)]) {
    fs::write(
        home.join(".direclaw/config.yaml"),
        serde_yaml::to_string(settings).expect("serialize settings"),
    )
    .expect("write settings");
    for (orchestrator_id, cfg) in configs {
        let path = orchestrator_path(settings, orchestrator_id);
        fs::create_dir_all(path.parent().expect("orchestrator parent"))
            .expect("mkdir orchestrator");
        fs::write(
            path,
            serde_yaml::to_string(cfg).expect("serialize orchestrator"),
        )
        .expect("write orchestrator");
    }
}

#[test]
fn setup_persists_orchestrator_config_at_resolved_workspace_path() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();
    let workspaces = home.join("workspace");
    let private_workspace = home.join("private-main");
    fs::create_dir_all(&workspaces).expect("create workspaces");
    fs::create_dir_all(home.join(".direclaw")).expect("create state root");
    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            "workspaces_path: {}\nshared_workspaces: {{}}\norchestrators:\n  main:\n    private_workspace: {}\n    shared_access: []\nchannel_profiles: {{}}\nmonitoring: {{}}\nchannels: {{}}\n",
            workspaces.display(),
            private_workspace.display()
        ),
    )
    .expect("write config");

    let output = run_setup(home);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let canonical = private_workspace.join("orchestrator.yaml");
    assert!(canonical.is_file());
    assert!(!workspaces.join("main/orchestrator.yaml").exists());
    let cfg: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&canonical).expect("read orchestrator"))
            .expect("parse orchestrator");
    assert_eq!(cfg.id, "main");
}

#[test]
fn setup_fails_when_existing_state_is_invalid_at_save_boundary() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();

    let first = run_setup(home);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let settings = Settings::from_path(&home.join(".direclaw/config.yaml")).expect("load settings");
    let orchestrator_path = settings
        .resolve_private_workspace("main")
        .expect("resolve workspace")
        .join("orchestrator.yaml");
    let mut cfg: OrchestratorConfig =
        serde_yaml::from_str(&fs::read_to_string(&orchestrator_path).expect("read orchestrator"))
            .expect("parse orchestrator");
    cfg.default_workflow = "missing_workflow".to_string();
    fs::write(
        &orchestrator_path,
        serde_yaml::to_string(&cfg).expect("serialize invalid orchestrator"),
    )
    .expect("write invalid orchestrator");

    let second = run_setup(home);
    assert!(!second.status.success());
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("default_workflow") || stderr.contains("missing_workflow"));
}

#[test]
fn setup_preserves_add_edit_delete_state_across_save_and_reload() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();

    let first = run_setup(home);
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let mut settings = load_settings(home);
    let mut main_cfg = load_orchestrator(&settings, "main");

    settings.orchestrators.insert(
        "alpha".to_string(),
        SettingsOrchestrator {
            private_workspace: Some(home.join("alpha-private")),
            shared_access: Vec::new(),
        },
    );

    let mut alpha_cfg = main_cfg.clone();
    alpha_cfg.id = "alpha".to_string();

    let default_agent = main_cfg
        .agents
        .get("default")
        .expect("default agent")
        .clone();
    let mut helper_agent = default_agent.clone();
    helper_agent.model = "helper-model-v2".to_string();
    helper_agent.can_orchestrate_workflows = true;
    main_cfg.agents.insert("helper".to_string(), helper_agent);
    main_cfg.selector_agent = "helper".to_string();
    main_cfg.agents.remove("default");

    let mut triage = main_cfg
        .workflows
        .iter()
        .find(|workflow| workflow.id == "default")
        .expect("default workflow")
        .clone();
    triage.id = "triage".to_string();
    triage.version = 2;
    let mut triage_step = triage.steps.first().expect("default step").clone();
    triage_step.id = "triage_step".to_string();
    triage_step.agent = "helper".to_string();
    triage_step.prompt = "Perform triage and summarize next action.".to_string();
    let mut triage_finalize = triage_step.clone();
    triage_finalize.id = "triage_finalize".to_string();
    triage_finalize.prompt = "Produce final structured response.".to_string();
    triage.steps = vec![triage_step, triage_finalize];
    triage.steps.retain(|step| step.id == "triage_finalize");

    main_cfg.workflows.push(triage);
    main_cfg.default_workflow = "triage".to_string();
    main_cfg
        .workflows
        .retain(|workflow| workflow.id == "triage");

    save_state(
        home,
        &settings,
        &[("main", &main_cfg), ("alpha", &alpha_cfg)],
    );

    let second = run_setup(home);
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let third = run_setup(home);
    assert!(
        third.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&third.stderr)
    );

    let reloaded_settings = load_settings(home);
    assert!(reloaded_settings.orchestrators.contains_key("main"));
    assert!(reloaded_settings.orchestrators.contains_key("alpha"));

    let reloaded_main = load_orchestrator(&reloaded_settings, "main");
    assert_eq!(reloaded_main.default_workflow, "triage");
    assert_eq!(reloaded_main.selector_agent, "helper");
    assert!(reloaded_main.agents.contains_key("helper"));
    assert!(!reloaded_main.agents.contains_key("default"));
    assert_eq!(reloaded_main.workflows.len(), 1);
    let triage = &reloaded_main.workflows[0];
    assert_eq!(triage.id, "triage");
    assert_eq!(triage.steps.len(), 1);
    assert_eq!(triage.steps[0].id, "triage_finalize");
    assert_eq!(triage.steps[0].agent, "helper");

    let reloaded_alpha = load_orchestrator(&reloaded_settings, "alpha");
    assert_eq!(reloaded_alpha.id, "alpha");
}

#[test]
fn setup_scripted_cancel_via_escape_does_not_persist_config() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();

    let output = run_setup_with_script_keys(home, "esc");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("setup canceled"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!home.join(".direclaw/config.yaml").exists());
}

#[test]
fn setup_scripted_cancel_via_ctrl_c_does_not_persist_config() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();

    let output = run_setup_with_script_keys(home, "ctrl-c");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("setup canceled"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!home.join(".direclaw/config.yaml").exists());
}

#[test]
fn setup_scripted_hotkeys_toggle_defaults_then_save() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();

    let output = run_setup_with_script_keys(home, "down,down,enter,t,esc,s");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("setup complete"), "stdout: {stdout}");
    assert!(stdout.contains("provider=openai"), "stdout: {stdout}");
    assert!(stdout.contains("model=gpt-5.3-codex"), "stdout: {stdout}");

    let settings = load_settings(home);
    assert!(settings.workspaces_path.exists());
}
