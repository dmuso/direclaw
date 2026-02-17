use direclaw::config::{ConfigError, Settings, ValidationOptions};
use std::fs;
use tempfile::tempdir;

#[test]
fn settings_memory_config_validates_when_cross_orchestrator_is_false() {
    let settings: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
memory:
  enabled: true
  bulletin_mode: every_message
  retrieval:
    top_n: 20
    rrf_k: 60
  ingest:
    enabled: true
    max_file_size_mb: 25
  scope:
    cross_orchestrator: false
"#,
    )
    .expect("parse settings");

    settings
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .expect("validation should pass");
}

#[test]
fn settings_memory_config_rejects_cross_orchestrator_true_in_v1() {
    let settings: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
memory:
  scope:
    cross_orchestrator: true
"#,
    )
    .expect("parse settings");

    let err = settings
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .expect_err("validation should fail");
    match err {
        ConfigError::Settings(message) => {
            assert!(message.contains("memory.scope.cross_orchestrator"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn settings_memory_config_rejects_unknown_memory_keys_explicitly() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.yaml");
    fs::write(
        &path,
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
memory:
  retrieval_top_n: 20
"#,
    )
    .expect("write config");

    let err = Settings::from_path(&path).expect_err("parse should fail");
    match err {
        ConfigError::Parse { source, .. } => {
            assert!(source.to_string().contains("unknown field"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn settings_memory_config_rejects_invalid_field_types() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.yaml");
    fs::write(
        &path,
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
memory:
  retrieval:
    top_n: "twenty"
"#,
    )
    .expect("write config");

    let err = Settings::from_path(&path).expect_err("parse should fail");
    assert!(matches!(err, ConfigError::Parse { .. }));
}
