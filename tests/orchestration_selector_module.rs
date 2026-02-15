use direclaw::config::OrchestratorConfig;
use direclaw::orchestration::selector::{
    parse_and_validate_selector_result, resolve_selector_with_retries,
    run_selector_attempt_with_provider, FunctionArgSchema, FunctionArgType, FunctionSchema,
    SelectorAction, SelectorRequest, SelectorStatus,
};
use direclaw::provider::RunnerBinaries;
use serde_json::Value;
use std::collections::BTreeMap;
use tempfile::tempdir;

fn sample_request() -> SelectorRequest {
    SelectorRequest {
        selector_id: "sel-1".to_string(),
        channel_profile_id: "engineering".to_string(),
        message_id: "m1".to_string(),
        conversation_id: Some("c1".to_string()),
        user_message: "status".to_string(),
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
