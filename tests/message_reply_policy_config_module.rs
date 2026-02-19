use direclaw::config::{Settings, ValidationOptions};
use tempfile::tempdir;

#[test]
fn slack_profile_validation_does_not_require_mention_identity_or_mention_flag() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspaces");
    std::fs::create_dir_all(&workspace).expect("workspace dir");

    let settings = serde_yaml::from_str::<Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  slack-main:
    channel: slack
    orchestrator_id: main
monitoring: {{}}
channels: {{}}
"#,
        workspace.display(),
        workspace.join("main").display()
    ))
    .expect("settings parse");

    settings
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .expect("validation should succeed without slack mention identity requirements");
}
