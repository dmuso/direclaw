use direclaw::config::{OrchestratorConfig, Settings};
use direclaw::orchestration::workspace_access::{
    enforce_workspace_access, resolve_workspace_access_context,
    verify_orchestrator_workspace_access, WorkspaceAccessContext,
};
use std::path::PathBuf;

fn sample_settings() -> Settings {
    serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspace
shared_workspaces:
  eng: /tmp/shared/eng
orchestrators:
  alpha:
    shared_access: [eng]
channel_profiles: {}
monitoring: {}
channels: {}
"#,
    )
    .expect("parse settings")
}

fn sample_orchestrator() -> OrchestratorConfig {
    serde_yaml::from_str(
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
    shared_access: [eng]
workflows:
  - id: wf
    version: 1
    inputs: [user_prompt]
    steps:
      - id: s1
        type: agent_task
        agent: router
        prompt: do work
        outputs: [result]
        output_files:
          result: out/result.txt
"#,
    )
    .expect("parse orchestrator")
}

#[test]
fn workspace_access_module_resolves_and_enforces_paths() {
    let settings = sample_settings();

    let context: WorkspaceAccessContext =
        resolve_workspace_access_context(&settings, "alpha").expect("context");
    assert_eq!(context.orchestrator_id, "alpha");

    enforce_workspace_access(
        &context,
        &[
            PathBuf::from("/tmp/workspace/alpha/agents/router"),
            PathBuf::from("/tmp/shared/eng/notes.md"),
        ],
    )
    .expect("allowed paths");

    let orchestrator = sample_orchestrator();
    let verified = verify_orchestrator_workspace_access(&settings, "alpha", &orchestrator)
        .expect("verify workspace access");
    assert_eq!(verified.orchestrator_id, "alpha");
}
