use direclaw::config::{ConfigError, OrchestratorConfig, Settings};
use direclaw::orchestration::error::OrchestratorError;
use direclaw::orchestration::workspace_access::{
    enforce_workspace_access, resolve_workspace_access_context,
};
use std::path::PathBuf;

#[test]
fn agent_shared_access_is_deny_by_default_without_orchestrator_grant() {
    let settings: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspace
shared_workspaces:
  common:
    path: /tmp/shared/common
    description: common shared workspace
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
    shared_access: [common]
workflows:
  - id: wf
    version: 1
    inputs: [user_prompt]
    steps:
      - id: s1
        type: agent_task
        agent: worker
        prompt: do work
        outputs: [summary, result]
        output_files:
          summary: out/summary.txt
          result: out/result.txt
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

#[test]
fn pre_execution_workspace_enforcement_isolated_per_orchestrator() {
    let settings: Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspace
shared_workspaces:
  eng:
    path: /tmp/shared/eng
    description: engineering shared workspace
  product:
    path: /tmp/shared/product
    description: product shared workspace
orchestrators:
  alpha:
    shared_access: [eng]
  beta:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
    )
    .expect("parse settings");

    let alpha = resolve_workspace_access_context(&settings, "alpha").expect("alpha context");
    let beta = resolve_workspace_access_context(&settings, "beta").expect("beta context");

    enforce_workspace_access(
        &alpha,
        &[
            PathBuf::from("/tmp/workspace/alpha/agents/worker"),
            PathBuf::from("/tmp/shared/eng/knowledge.md"),
        ],
    )
    .expect("alpha can access private+granted shared");

    let denied = enforce_workspace_access(&beta, &[PathBuf::from("/tmp/shared/eng/knowledge.md")])
        .expect_err("beta should not access shared workspace without grants");
    match denied {
        OrchestratorError::WorkspaceAccessDenied { path, .. } => {
            assert!(path.contains("/tmp/shared/eng/knowledge.md"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
