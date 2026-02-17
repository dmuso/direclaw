use direclaw::config::{ConfigError, OrchestratorConfig, Settings};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn config_save_module_exposes_persistence_entry_points() {
    let _save_settings: fn(&Settings) -> Result<PathBuf, ConfigError> =
        direclaw::config::save::save_settings;
    let _save_orchestrator: fn(
        &Settings,
        &str,
        &OrchestratorConfig,
    ) -> Result<PathBuf, ConfigError> = direclaw::config::save::save_orchestrator_config;
    let _save_registry: fn(
        &Settings,
        &BTreeMap<String, OrchestratorConfig>,
    ) -> Result<PathBuf, ConfigError> = direclaw::config::save::save_orchestrator_registry;
    let _remove_orchestrator: fn(&Settings, &str) -> Result<(), ConfigError> =
        direclaw::config::save::remove_orchestrator_config;
}

#[test]
fn config_save_module_save_orchestrator_bootstraps_memory_runtime_paths() {
    let dir = tempdir().expect("tempdir");
    let workspace = dir.path().join("workspaces");
    fs::create_dir_all(&workspace).expect("workspace");

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  alpha:
    private_workspace: {workspace}/alpha
    shared_access: []
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
        workspace = workspace.display()
    ))
    .expect("parse settings");

    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: alpha
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
    description: triage workflow
    tags: [triage]
    inputs: [user_prompt]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
        outputs: [summary]
        output_files:
          summary: summary.txt
"#,
    )
    .expect("parse orchestrator");

    let path = direclaw::config::save::save_orchestrator_config(&settings, "alpha", &orchestrator)
        .expect("save orchestrator");
    assert!(
        path.is_file(),
        "missing orchestrator config at {}",
        path.display()
    );

    let runtime_root = workspace.join("alpha");
    assert!(runtime_root.join("memory/ingest").is_dir());
    assert!(runtime_root.join("memory/ingest/processed").is_dir());
    assert!(runtime_root.join("memory/ingest/rejected").is_dir());
    assert!(runtime_root.join("memory/bulletins").is_dir());
    assert!(runtime_root.join("memory/logs").is_dir());
}
