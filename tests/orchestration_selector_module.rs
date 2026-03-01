use direclaw::config::OrchestratorConfig;
use direclaw::orchestration::selector::{
    parse_and_validate_selector_result, resolve_orchestrator_id, resolve_selector_with_retries,
    run_selector_attempt_with_provider, FunctionArgSchema, FunctionArgType, FunctionSchema,
    SelectorAction, SelectorRequest, SelectorStatus,
};
use direclaw::provider::RunnerBinaries;
use direclaw::queue::IncomingMessage;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

fn sample_request() -> SelectorRequest {
    SelectorRequest {
        selector_id: "sel-1".to_string(),
        channel_profile_id: "engineering".to_string(),
        message_id: "m1".to_string(),
        conversation_id: Some("c1".to_string()),
        user_message: "/workflow.status run-1".to_string(),
        thread_context: None,
        memory_bulletin: None,
        memory_bulletin_citations: Vec::new(),
        available_workflows: vec!["default".to_string()],
        default_workflow: "default".to_string(),
        available_functions: vec!["workflow.status".to_string()],
        available_function_schemas: vec![FunctionSchema {
            function_id: "workflow.status".to_string(),
            description: "show status".to_string(),
            args: BTreeMap::from([(
                "runId".to_string(),
                FunctionArgSchema {
                    arg_type: FunctionArgType::String,
                    required: true,
                    description: "Run id".to_string(),
                },
            )]),
            read_only: true,
        }],
    }
}

fn sample_orchestrator() -> OrchestratorConfig {
    OrchestratorConfig {
        id: "eng".to_string(),
        selector_agent: "selector".to_string(),
        default_workflow: "default".to_string(),
        selection_max_retries: 1,
        selector_timeout_seconds: 30,
        agents: BTreeMap::new(),
        workflows: Vec::new(),
        workflow_orchestration: None,
    }
}

fn write_script(path: &std::path::Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

#[test]
fn selector_module_validates_and_retries() {
    let request = sample_request();

    let raw = r#"{
      "selectorId":"sel-1",
      "status":"selected",
      "action":"command_invoke",
      "functionId":"workflow.status",
      "functionArgs":{"runId":"run-1"}
    }"#;

    let parsed = parse_and_validate_selector_result(raw, &request).expect("valid selector");
    assert_eq!(parsed.status, SelectorStatus::Selected);
    assert_eq!(parsed.action, Some(SelectorAction::CommandInvoke));

    let orchestrator = sample_orchestrator();
    let selection = resolve_selector_with_retries(&orchestrator, &request, |attempt| {
        if attempt == 0 {
            Some("{}".to_string())
        } else {
            Some(raw.to_string())
        }
    });

    assert!(!selection.fell_back_to_default_workflow);
    assert_eq!(selection.retries_used, 1);
    assert_eq!(
        selection
            .result
            .function_args
            .as_ref()
            .and_then(|args| args.get("runId")),
        Some(&Value::String("run-1".to_string()))
    );
}

#[test]
fn selector_module_rejects_unknown_argument() {
    let mut request = sample_request();
    request.available_function_schemas[0].args = BTreeMap::from([(
        "runId".to_string(),
        FunctionArgSchema {
            arg_type: FunctionArgType::String,
            required: true,
            description: "Run id".to_string(),
        },
    )]);

    let raw = r#"{
      "selectorId":"sel-1",
      "status":"selected",
      "action":"command_invoke",
      "functionId":"workflow.status",
      "functionArgs":{"bogus":"run-1"}
    }"#;

    let err = parse_and_validate_selector_result(raw, &request).expect_err("must fail");
    assert!(err
        .to_string()
        .contains("unknown argument `bogus` for function `workflow.status`"));
}

#[test]
fn selector_module_rejects_command_invoke_without_slash_command() {
    let mut request = sample_request();
    request.user_message = "workflow.status run-1".to_string();
    let raw = r#"{
      "selectorId":"sel-1",
      "status":"selected",
      "action":"command_invoke",
      "functionId":"workflow.status",
      "functionArgs":{"runId":"run-1"}
    }"#;

    let err = parse_and_validate_selector_result(raw, &request).expect_err("must fail");
    assert!(err.to_string().contains("requires explicit slash command"));
}

#[test]
fn selector_module_accepts_no_response_action() {
    let request = sample_request();
    let raw = r#"{
      "selectorId":"sel-1",
      "status":"selected",
      "action":"no_response",
      "reason":"low_value_thread_noise"
    }"#;
    let parsed = parse_and_validate_selector_result(raw, &request).expect("valid selector");
    assert_eq!(parsed.status, SelectorStatus::Selected);
    assert_eq!(parsed.action, Some(SelectorAction::NoResponse));
}

#[test]
fn selector_module_rejects_diagnostics_investigate_action() {
    let request = sample_request();
    let raw = r#"{
      "selectorId":"sel-1",
      "status":"selected",
      "action":"diagnostics_investigate",
      "diagnosticsScope":{"runId":"run-1"}
    }"#;
    let err = parse_and_validate_selector_result(raw, &request).expect_err("must fail");
    assert!(err
        .to_string()
        .contains("unknown variant `diagnostics_investigate`"));
}

#[test]
fn selector_module_exposes_selector_provider_attempt_runner() {
    let settings: direclaw::config::Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators:
  eng:
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: eng
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {}
channels: {}
"#,
    )
    .expect("settings");
    let request = sample_request();
    let temp = tempdir().expect("tempdir");
    let binaries = RunnerBinaries::default();

    let err = run_selector_attempt_with_provider(
        temp.path(),
        &settings,
        &request,
        &sample_orchestrator(),
        0,
        &binaries,
    )
    .expect_err("selector agent must exist");

    assert!(err.contains("selector agent `selector` missing"));
}

#[test]
fn selector_module_writes_prompt_artifacts_under_orchestrator_artifacts() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");

    let selector_bin = temp.path().join("claude-mock");
    write_script(
        &selector_bin,
        r#"#!/bin/sh
set -eu
msg="$*"
result_path=$(printf "%s" "$msg" | sed -n 's/.*Write selector result JSON to: \([^ ]*\).*/\1/p')
selector_id=$(basename "$result_path" | sed -E 's/^selector-provider-result-(.*)_attempt_[0-9]+\.json$/\1/')
printf '{"selectorId":"%s","status":"selected","action":"workflow_start","selectedWorkflow":"default"}' "$selector_id" > "$result_path"
echo ok
"#,
    );

    let settings: direclaw::config::Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspaces}
shared_workspaces: {{}}
orchestrators:
  eng:
    private_workspace: {workspace}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: eng
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspaces = temp.path().display(),
        workspace = workspace.display()
    ))
    .expect("settings");

    let orchestrator: OrchestratorConfig = serde_yaml::from_str(
        r#"
id: eng
selector_agent: selector
default_workflow: default
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  selector:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
workflows:
  - id: default
    version: 1
    steps: []
"#,
    )
    .expect("orchestrator");

    let result = run_selector_attempt_with_provider(
        temp.path(),
        &settings,
        &sample_request(),
        &orchestrator,
        1,
        &RunnerBinaries {
            anthropic: selector_bin.display().to_string(),
            openai: selector_bin.display().to_string(),
        },
    )
    .expect("selector result");

    assert!(result.contains("\"workflow_start\""));
    assert!(workspace
        .join("orchestrator/artifacts/selector/sel-1/attempts/1/prompt.md")
        .is_file());
    assert!(workspace
        .join("orchestrator/artifacts/selector/sel-1/attempts/1/context.md")
        .is_file());
    assert!(!workspace.join("prompt.md").is_file());
    assert!(!workspace.join("context.md").is_file());
}

#[test]
fn selector_module_resolves_heartbeat_orchestrator_without_channel_profile_id() {
    let settings: direclaw::config::Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspaces
shared_workspaces: {}
orchestrators:
  orch:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
    )
    .expect("settings");

    let inbound = IncomingMessage {
        channel: "heartbeat".to_string(),
        channel_profile_id: None,
        sender: "heartbeat:orch".to_string(),
        sender_id: "heartbeat-agent".to_string(),
        message: "health".to_string(),
        timestamp: 1,
        message_id: "hb-1".to_string(),
        conversation_id: Some("hb:orch:agent".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: Vec::new(),
        workflow_run_id: Some("hb:orch:agent".to_string()),
        workflow_step_id: Some("heartbeat_worker_check".to_string()),
    };

    let orchestrator_id = resolve_orchestrator_id(&settings, &inbound).expect("resolved");
    assert_eq!(orchestrator_id, "orch");
}
