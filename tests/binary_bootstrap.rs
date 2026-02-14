use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn binary_loads_global_config_and_bootstraps_default_state_root() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path();
    let workspace = home.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::write(
        home.join(".direclaw.yaml"),
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

    let settings = direclaw::config::Settings::from_path(&home.join(".direclaw.yaml"))
        .expect("load generated config");
    let expected_workspace = home.join(".direclaw/workspace");
    assert_eq!(settings.workspace_path, expected_workspace);
    assert!(expected_workspace.is_dir());
}
