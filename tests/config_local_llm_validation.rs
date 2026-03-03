use direclaw::config::{ConfigError, Settings, ValidationOptions};
use direclaw::local_llm::LocalLlmProvider;

#[test]
fn settings_local_llm_defaults_parse_and_validate() {
    let settings: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
"#,
    )
    .expect("parse settings");

    settings
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .expect("validation succeeds");
    assert!(!settings.local_llm.enabled);
    assert_eq!(settings.local_llm.provider, LocalLlmProvider::LlamaCpp);
}

#[test]
fn settings_local_llm_rejects_unknown_keys() {
    let err = serde_yaml::from_str::<Settings>(
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
local_llm:
  enabled: true
  unknown_field: true
"#,
    )
    .expect_err("parse should fail");

    assert!(err.to_string().contains("unknown field"));
}

#[test]
fn settings_local_llm_rejects_invalid_top_p_bounds() {
    let settings: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
local_llm:
  enabled: true
  inference:
    top_p: 0.0
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
            assert!(message.contains("local_llm.inference.top_p"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn settings_local_llm_rejects_invalid_generation_timeout() {
    let settings: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
local_llm:
  enabled: true
  inference:
    max_generation_millis: 0
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
            assert!(message.contains("local_llm.inference.max_generation_millis"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
