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
workspaces_path: {}
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
    assert!(state_root.join("logs").is_dir());
    assert!(state_root.join("runtime/preferences.yaml").is_file());
    assert!(!state_root.join("queue").exists());
    assert!(!state_root.join("workflows/runs").exists());
    assert!(!state_root.join("orchestrator").exists());
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
    let expected_workspace = home.join(".direclaw/workspaces");
    assert_eq!(settings.workspaces_path, expected_workspace);
    assert!(expected_workspace.is_dir());
    assert!(settings.orchestrators.contains_key("main"));

    let orchestrator_raw =
        fs::read_to_string(home.join(".direclaw/workspaces/main/orchestrator.yaml"))
            .expect("read generated orchestrator config");
    let orchestrator: direclaw::config::OrchestratorConfig =
        serde_yaml::from_str(&orchestrator_raw).expect("parse orchestrator config");
    assert_eq!(orchestrator.id, "main");
    assert_eq!(orchestrator.default_workflow, "default");
    let default_workflow = orchestrator
        .workflows
        .iter()
        .find(|workflow| workflow.id == "default")
        .expect("default workflow");
    let first_step = default_workflow.steps.first().expect("default step");
    assert!(first_step.prompt.ends_with(".prompt.md"));
    assert_eq!(
        first_step.prompt_type,
        direclaw::config::WorkflowStepPromptType::FileOutput
    );
    let prompt_path = home
        .join(".direclaw/workspaces/main/prompts")
        .join(&first_step.prompt);
    let prompt_body = fs::read_to_string(prompt_path).expect("read prompt template");
    assert!(prompt_body.contains("When complete, write structured output"));
    assert!(prompt_body.contains("Required outputs schema:"));
    assert!(prompt_body.contains("{{workflow.output_schema_json}}"));
    assert!(prompt_body.contains("{{workflow.output_paths.summary}}"));
    assert!(prompt_body.contains("{{workflow.output_paths.artifact}}"));
    assert!(!prompt_body.contains("You are the default workflow step."));
    assert!(!first_step.outputs.is_empty());
    assert!(!first_step.output_files.is_empty());

    let prefs_path = home.join(".direclaw/runtime/preferences.yaml");
    assert!(prefs_path.is_file());
}
