use direclaw::config::{ConfigError, OrchestratorConfig, Settings};

#[test]
fn agent_shared_access_is_deny_by_default_without_orchestrator_grant() {
    let settings: Settings = serde_yaml::from_str(
        r#"
workspace_path: /tmp/workspace
shared_workspaces:
  docs: /tmp/docs
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
default_workflow: wf
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
    shared_access: []
  worker:
    provider: openai
    model: gpt-5.2
    can_orchestrate_workflows: false
    shared_access: [docs]
workflows:
  - id: wf
    version: 1
    inputs: [user_prompt]
    steps:
      - id: s1
        type: agent_task
        agent: worker
        prompt: do work
"#,
    )
    .expect("parse orchestrator");

    let err = config
        .validate(&settings, "alpha")
        .expect_err("validation should fail");

    match err {
        ConfigError::Orchestrator(message) => {
            assert!(message.contains("is not granted to orchestrator"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
