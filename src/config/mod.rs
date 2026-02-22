pub mod error;
pub mod load;
pub mod orchestrator_file;
pub mod orchestrators_registry;
pub mod paths;
pub mod save;
pub mod settings;
pub(crate) mod setup_draft;
pub mod typed_fields;
pub mod validate;
pub use crate::memory::{
    MemoryBulletinMode, MemoryConfig, MemoryIngestConfig, MemoryRetrievalConfig, MemoryScopeConfig,
};
pub use error::ConfigError;
pub use load::{load_global_settings, load_orchestrator_config};
pub use orchestrator_file::{
    agent_editable_fields, AgentConfig, AgentEditableField, ConfigProviderKind, OrchestratorConfig,
    StepLimitsConfig, WorkflowConfig, WorkflowLimitsConfig, WorkflowOrchestrationConfig,
    WorkflowStepConfig, WorkflowStepPromptType, WorkflowStepType, WorkflowStepWorkspaceMode,
};
pub use orchestrators_registry::{remove_orchestrator_config, save_orchestrator_registry};
pub use paths::{
    default_global_config_path, default_orchestrators_config_path, GLOBAL_ORCHESTRATORS_FILE_NAME,
    GLOBAL_SETTINGS_FILE_NAME, GLOBAL_STATE_DIR,
};
pub use save::{save_orchestrator_config, save_settings};
pub use settings::{
    AuthSyncConfig, AuthSyncSource, ChannelConfig, ChannelKind, ChannelProfile,
    ChannelProfileIdentity, Monitoring, Settings, SettingsOrchestrator, SharedWorkspaceConfig,
    SlackInboundMode, ThreadResponseMode, ValidationOptions,
};
pub(crate) use setup_draft::{OrchestrationLimitField, SetupDraft};
pub use typed_fields::{
    normalize_workflow_input_key, parse_output_contract_key, AgentId, OrchestratorId,
    OutputContractKey, OutputKey, PathTemplate, StepId, WorkflowId, WorkflowInputKey,
    WorkflowInputs, WorkflowTag,
};
pub use validate::{validate_orchestrator_config, validate_settings};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn private_workspace_override_wins() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    private_workspace: /tmp/custom-alpha
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let resolved = settings
            .resolve_private_workspace("alpha")
            .expect("resolve workspace");
        assert_eq!(resolved, PathBuf::from("/tmp/custom-alpha"));
    }

    #[test]
    fn private_workspace_falls_back_to_default_rule() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let resolved = settings
            .resolve_private_workspace("alpha")
            .expect("resolve workspace");
        assert_eq!(resolved, PathBuf::from("/tmp/workspace/alpha"));
    }

    #[test]
    fn orchestrator_runtime_root_scopes_under_private_workspace() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    private_workspace: /tmp/custom-alpha
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let resolved = settings
            .resolve_orchestrator_runtime_root("alpha")
            .expect("resolve runtime root");
        assert_eq!(resolved, PathBuf::from("/tmp/custom-alpha"));
    }

    #[test]
    fn channel_profile_runtime_root_uses_profile_orchestrator_scope() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    private_workspace: /tmp/custom-alpha
    shared_access: []
channel_profiles:
  local-default:
    channel: local
    orchestrator_id: alpha
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let resolved = settings
            .resolve_channel_profile_runtime_root("local-default")
            .expect("resolve runtime root");
        assert_eq!(resolved, PathBuf::from("/tmp/custom-alpha"));
    }

    #[test]
    fn settings_validation_fails_for_unknown_shared_grant() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces:
  docs:
    path: /tmp/docs
    description: shared docs
orchestrators:
  alpha:
    shared_access: [missing]
channel_profiles: {}
monitoring: {}
channels: {}
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
                assert!(message.contains("unknown shared workspace"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn settings_validation_fails_for_blank_shared_workspace_description() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces:
  docs:
    path: /tmp/docs
    description: "   "
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
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
                assert!(message.contains("description must be non-empty"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn settings_validation_canonicalizes_and_requires_shared_paths_when_enabled() {
        let temp = tempdir().expect("temp dir");
        let docs = temp.path().join("docs");
        fs::create_dir_all(&docs).expect("create docs path");

        let yaml = format!(
            r#"
workspaces_path: {workspace}
shared_workspaces:
  docs:
    path: {docs}
    description: shared docs
orchestrators:
  alpha:
    shared_access: [docs]
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
            workspace = temp.path().display(),
            docs = docs.display()
        );

        let settings: Settings = serde_yaml::from_str(&yaml).expect("parse settings");
        settings
            .validate(ValidationOptions::default())
            .expect("validation succeeds");
    }

    #[test]
    fn orchestrator_validation_enforces_selector_default_and_workflows() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let config: OrchestratorConfig = serde_yaml::from_str(
            r#"
id: alpha
selector_agent: router
default_workflow: missing
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
workflows:
  - id: real
    version: 1
    inputs: [user_prompt]
    steps:
      - id: step_1
        type: agent_task
        agent: router
        prompt: hello
        outputs: [summary]
        output_files:
          summary: summary.txt
"#,
        )
        .expect("parse orchestrator");

        let err = config
            .validate(&settings, "alpha")
            .expect_err("validation should fail");
        match err {
            ConfigError::Orchestrator(message) => {
                assert!(message.contains("default_workflow"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn orchestrator_validation_rejects_zero_selector_timeout_seconds() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let config: OrchestratorConfig = serde_yaml::from_str(
            r#"
id: alpha
selector_agent: router
default_workflow: real
selection_max_retries: 1
selector_timeout_seconds: 0
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
workflows:
  - id: real
    version: 1
    steps:
      - id: step_1
        type: agent_task
        agent: router
        prompt: hello
        outputs: [summary]
        output_files:
          summary: summary.txt
"#,
        )
        .expect("parse orchestrator");

        let err = config
            .validate(&settings, "alpha")
            .expect_err("validation should fail");
        match err {
            ConfigError::Orchestrator(message) => {
                assert!(message.contains("selector_timeout_seconds"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn orchestrator_validation_rejects_output_keys_with_non_trailing_optional_marker() {
        let _settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let err = serde_yaml::from_str::<OrchestratorConfig>(
            r#"
id: alpha
selector_agent: router
default_workflow: real
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
workflows:
  - id: real
    version: 1
    steps:
      - id: step_1
        type: agent_task
        agent: router
        prompt: hello
        outputs: [plan?draft]
        output_files:
          "plan?draft": plan.md
"#,
        )
        .expect_err("invalid output key should fail at parse");
        let message = err.to_string();
        assert!(message.contains("output key"));
        assert!(message.contains("trailing `?`"));
    }

    #[test]
    fn default_global_config_path_targets_home_direclaw_config_yaml() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("temp dir");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

        let path = default_global_config_path().expect("resolve global config path");
        assert_eq!(path, temp.path().join(".direclaw/config.yaml"));

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn load_global_settings_reads_direclaw_config_yaml() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let temp = tempdir().expect("temp dir");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(temp.path().join(".direclaw")).expect("create config dir");

        let config_path = temp.path().join(".direclaw/config.yaml");
        fs::write(
            &config_path,
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

        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());
        let settings = load_global_settings().expect("load global settings");
        assert_eq!(settings.workspaces_path, workspace);
        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn workflow_step_workspace_mode_defaults_to_orchestrator_workspace() {
        let step: WorkflowStepConfig = serde_yaml::from_str(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
outputs: [summary]
output_files:
  summary: outputs/summary.txt
"#,
        )
        .expect("parse step");
        assert_eq!(
            step.workspace_mode,
            WorkflowStepWorkspaceMode::OrchestratorWorkspace
        );
    }

    #[test]
    fn workflow_step_workspace_mode_accepts_supported_values_and_rejects_unknown() {
        let run_mode: WorkflowStepConfig = serde_yaml::from_str(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
workspace_mode: run_workspace
outputs: [summary]
output_files:
  summary: outputs/summary.txt
"#,
        )
        .expect("parse run_workspace");
        assert_eq!(
            run_mode.workspace_mode,
            WorkflowStepWorkspaceMode::RunWorkspace
        );

        let err = serde_yaml::from_str::<WorkflowStepConfig>(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
workspace_mode: unknown_mode
outputs: [summary]
output_files:
  summary: outputs/summary.txt
"#,
        )
        .expect_err("unknown workspace_mode must fail");
        assert!(err.to_string().contains("workspace_mode"));

        let agent_mode = serde_yaml::from_str::<WorkflowStepConfig>(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
workspace_mode: agent_workspace
outputs: [summary]
output_files:
  summary: outputs/summary.txt
"#,
        )
        .expect_err("agent_workspace mode is no longer supported");
        assert!(agent_mode.to_string().contains("agent_workspace"));
    }

    #[test]
    fn orchestrator_agent_legacy_fields_are_rejected() {
        let err = serde_yaml::from_str::<OrchestratorConfig>(
            r#"
id: alpha
selector_agent: router
default_workflow: wf
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
    private_workspace: /tmp/legacy-agent-workspace
workflows:
  - id: wf
    version: 1
    description: workflow
    tags: [test]
    inputs: [user_prompt]
    steps:
      - id: s1
        type: agent_task
        agent: router
        prompt: do work
        outputs: [summary]
        output_files:
          summary: out/summary.txt
"#,
        )
        .expect_err("legacy private_workspace field must fail parsing");
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("private_workspace"));
    }

    #[test]
    fn workflow_step_final_output_priority_defaults_to_artifact_then_summary() {
        let step: WorkflowStepConfig = serde_yaml::from_str(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
outputs: [summary, artifact]
output_files:
  summary: outputs/summary.txt
  artifact: outputs/artifact.txt
"#,
        )
        .expect("parse step");
        let priority = step
            .final_output_priority
            .iter()
            .map(|key| key.as_str().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            priority,
            vec!["artifact".to_string(), "summary".to_string()]
        );
    }

    #[test]
    fn orchestrator_validation_rejects_final_output_priority_key_not_declared_in_outputs() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let config: OrchestratorConfig = serde_yaml::from_str(
            r#"
id: alpha
selector_agent: router
default_workflow: real
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
workflows:
  - id: real
    version: 1
    steps:
      - id: step_1
        type: agent_task
        agent: router
        prompt: hello
        outputs: [summary]
        output_files:
          summary: summary.txt
        final_output_priority: [artifact, summary]
"#,
        )
        .expect("parse orchestrator");

        let err = config
            .validate(&settings, "alpha")
            .expect_err("validation should fail");
        match err {
            ConfigError::Orchestrator(message) => {
                assert!(message.contains("final_output_priority"));
                assert!(message.contains("artifact"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn workflow_inputs_round_trip_and_normalize_keys() {
        let workflow: WorkflowConfig = serde_yaml::from_str(
            r#"
id: triage
version: 1
inputs: [ ticket ,priority,ticket]
steps:
  - id: step_1
    type: agent_task
    agent: worker
    prompt: hello
    outputs: [summary]
    output_files:
      summary: outputs/summary.txt
"#,
        )
        .expect("parse workflow");
        let keys = workflow
            .inputs
            .as_slice()
            .iter()
            .map(|key| key.as_str().to_string())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["ticket".to_string(), "priority".to_string()]);

        let encoded = serde_yaml::to_string(&workflow).expect("encode workflow");
        assert!(encoded.contains("- ticket"));
        assert!(encoded.contains("- priority"));
    }

    #[test]
    fn workflow_inputs_reject_mapping_shape() {
        let err = serde_yaml::from_str::<WorkflowConfig>(
            r#"
id: triage
version: 1
inputs:
  ticket: true
  priority: high
steps:
  - id: step_1
    type: agent_task
    agent: worker
    prompt: hello
    outputs: [summary]
    output_files:
      summary: outputs/summary.txt
"#,
        )
        .expect_err("mapping inputs should fail");
        assert!(err.to_string().contains("sequence of string keys"));
    }

    #[test]
    fn workflow_inputs_reject_invalid_key_shapes() {
        let err = serde_yaml::from_str::<WorkflowConfig>(
            r#"
id: triage
version: 1
inputs: ["bad key"]
steps:
  - id: step_1
    type: agent_task
    agent: worker
    prompt: hello
    outputs: [summary]
    output_files:
      summary: outputs/summary.txt
"#,
        )
        .expect_err("invalid workflow input key should fail");
        assert!(err.to_string().contains("workflow input key"));
    }

    #[test]
    fn workflow_step_requires_outputs_and_output_files_fields() {
        let err = serde_yaml::from_str::<WorkflowStepConfig>(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
"#,
        )
        .expect_err("missing output contract fields must fail");
        let message = err.to_string();
        assert!(message.contains("outputs"));
        assert!(message.contains("output_files"));
    }

    #[test]
    fn output_contract_key_parsing_tracks_required_and_optional_markers() {
        let required = parse_output_contract_key("summary").expect("required output key");
        assert_eq!(required.name, "summary");
        assert!(required.required);

        let optional = parse_output_contract_key("artifact?").expect("optional output key");
        assert_eq!(optional.name, "artifact");
        assert!(!optional.required);

        let err = parse_output_contract_key("art?ifact")
            .expect_err("non-trailing optional marker should fail");
        assert!(err.contains("trailing `?`"));
    }

    #[test]
    fn typed_enums_round_trip_with_snake_case_yaml() {
        let agent: AgentConfig = serde_yaml::from_str(
            r#"
provider: openai
model: gpt-5.3-codex
can_orchestrate_workflows: false
"#,
        )
        .expect("parse agent");
        assert_eq!(agent.provider, ConfigProviderKind::OpenAi);
        let encoded = serde_yaml::to_string(&agent).expect("encode agent");
        assert!(encoded.contains("provider: openai"));

        let profile: ChannelProfile = serde_yaml::from_str(
            r#"
channel: slack
orchestrator_id: main
slack_app_user_id: U123
require_mention_in_channels: true
"#,
        )
        .expect("parse profile");
        assert_eq!(profile.channel, ChannelKind::Slack);
        let encoded = serde_yaml::to_string(&profile).expect("encode profile");
        assert!(encoded.contains("channel: slack"));

        let step: WorkflowStepConfig = serde_yaml::from_str(
            r#"
id: review
type: agent_review
agent: reviewer
prompt: review it
outputs: [decision,summary,feedback]
output_files:
  decision: outputs/decision.txt
  summary: outputs/summary.txt
  feedback: outputs/feedback.txt
"#,
        )
        .expect("parse step");
        assert_eq!(step.step_type, WorkflowStepType::AgentReview);
        let encoded = serde_yaml::to_string(&step).expect("encode step");
        assert!(encoded.contains("type: agent_review"));
    }

    #[test]
    fn typed_enums_reject_invalid_values_with_parse_errors() {
        let provider_err = serde_yaml::from_str::<AgentConfig>(
            r#"
provider: invalid
model: sonnet
"#,
        )
        .expect_err("invalid provider should fail");
        assert!(provider_err.to_string().contains("provider"));

        let channel_err = serde_yaml::from_str::<ChannelProfile>(
            r#"
channel: invalid
orchestrator_id: main
"#,
        )
        .expect_err("invalid channel should fail");
        assert!(channel_err.to_string().contains("channel"));

        let step_err = serde_yaml::from_str::<WorkflowStepConfig>(
            r#"
id: s1
type: invalid
agent: worker
prompt: test
"#,
        )
        .expect_err("invalid step type should fail");
        assert!(step_err.to_string().contains("type"));
    }

    #[test]
    fn id_wrappers_accept_valid_and_reject_invalid_values() {
        assert!(OrchestratorId::parse("main_01").is_ok());
        assert!(WorkflowId::parse("feature-delivery").is_ok());
        assert!(StepId::parse("step_1").is_ok());
        assert!(AgentId::parse("router").is_ok());

        assert!(OrchestratorId::parse("main dev").is_err());
        assert!(WorkflowId::parse("").is_err());
        assert!(StepId::parse("step!").is_err());
        assert!(AgentId::parse("agent/id").is_err());
    }
}
