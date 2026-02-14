use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn binary_loads_global_config_and_bootstraps_default_state_root() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();
    let workspace = home.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("create config dir");
    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspace_path: {}
shared_workspaces: {{}}
orchestrators: {{}}
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
            workspace.display()
        ),
    )
    .expect("write global config");

    let output = Command::new(env!("CARGO_BIN_EXE_direclaw"))
        .arg("setup")
        .env("HOME", home)
        .output()
        .expect("run binary");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let state_root = home.join(".direclaw");
    assert!(state_root.join("queue/incoming").is_dir());
    assert!(state_root.join("queue/processing").is_dir());
    assert!(state_root.join("queue/outgoing").is_dir());
    assert!(state_root.join("workflows/runs").is_dir());
}

#[test]
fn setup_defaults_workspace_under_state_root_when_config_is_missing() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();

    let output = Command::new(env!("CARGO_BIN_EXE_direclaw"))
        .arg("setup")
        .env("HOME", home)
        .output()
        .expect("run binary");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let settings = direclaw::config::Settings::from_path(&home.join(".direclaw/config.yaml"))
        .expect("load generated config");
    let expected_workspace = home.join(".direclaw/workspace");
    assert_eq!(settings.workspace_path, expected_workspace);
    assert!(expected_workspace.is_dir());
    assert!(settings.orchestrators.contains_key("main"));

    let orchestrators_raw = fs::read_to_string(home.join(".direclaw/config-orchestrators.yaml"))
        .expect("read generated orchestrator registry");
    let orchestrators: std::collections::BTreeMap<String, direclaw::config::OrchestratorConfig> =
        serde_yaml::from_str(&orchestrators_raw).expect("parse orchestrator registry");
    let orchestrator = orchestrators.get("main").expect("main orchestrator");
    assert_eq!(orchestrator.id, "main");
    assert_eq!(orchestrator.default_workflow, "default");

    let prefs_path = home.join(".direclaw/runtime/preferences.yaml");
    assert!(prefs_path.is_file());
}
