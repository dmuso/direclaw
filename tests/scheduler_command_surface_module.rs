use direclaw::app::command_catalog::function_ids;
use direclaw::orchestration::function_registry::FunctionRegistry;
use direclaw::orchestration::selector::{
    parse_and_validate_selector_result, FunctionSchema, SelectorRequest,
};
use serde_json::Value;
use std::collections::BTreeSet;

#[test]
fn scheduler_functions_are_exposed_in_v1_registry() {
    let ids = FunctionRegistry::new(vec![
        function_ids::SCHEDULE_CREATE.to_string(),
        function_ids::SCHEDULE_LIST.to_string(),
        function_ids::SCHEDULE_SHOW.to_string(),
        function_ids::SCHEDULE_UPDATE.to_string(),
        function_ids::SCHEDULE_PAUSE.to_string(),
        function_ids::SCHEDULE_RESUME.to_string(),
        function_ids::SCHEDULE_DELETE.to_string(),
        function_ids::SCHEDULE_RUN_NOW.to_string(),
    ])
    .available_function_ids()
    .into_iter()
    .collect::<BTreeSet<_>>();

    for expected in [
        function_ids::SCHEDULE_CREATE,
        function_ids::SCHEDULE_LIST,
        function_ids::SCHEDULE_SHOW,
        function_ids::SCHEDULE_UPDATE,
        function_ids::SCHEDULE_PAUSE,
        function_ids::SCHEDULE_RESUME,
        function_ids::SCHEDULE_DELETE,
        function_ids::SCHEDULE_RUN_NOW,
    ] {
        assert!(ids.contains(expected), "missing function `{expected}`");
    }
}

#[test]
fn selector_validation_rejects_unknown_scheduler_args() {
    let registry = FunctionRegistry::new(vec![function_ids::SCHEDULE_PAUSE.to_string()]);
    let schemas: Vec<FunctionSchema> = registry.available_function_schemas();

    let request = SelectorRequest {
        selector_id: "sel-scheduler-1".to_string(),
        channel_profile_id: "eng".to_string(),
        message_id: "m1".to_string(),
        conversation_id: Some("thread-1".to_string()),
        user_message: "/schedule.pause job-1".to_string(),
        thread_context: None,
        memory_bulletin: None,
        memory_bulletin_citations: vec![],
        available_workflows: vec![],
        default_workflow: "default".to_string(),
        available_functions: vec![function_ids::SCHEDULE_PAUSE.to_string()],
        available_function_schemas: schemas,
    };

    let err = parse_and_validate_selector_result(
        r#"{
          "selectorId":"sel-scheduler-1",
          "status":"selected",
          "action":"command_invoke",
          "functionId":"schedule.pause",
          "functionArgs":{"jobId":"job-1","extra":"nope"}
        }"#,
        &request,
    )
    .expect_err("unknown scheduler arg should fail");

    assert!(err.to_string().contains("unknown argument `extra`"));

    let ok = parse_and_validate_selector_result(
        r#"{
          "selectorId":"sel-scheduler-1",
          "status":"selected",
          "action":"command_invoke",
          "functionId":"schedule.pause",
          "functionArgs":{"jobId":"job-1"}
        }"#,
        &request,
    )
    .expect("valid scheduler invocation");

    assert_eq!(
        ok.function_args
            .and_then(|args| args.get("jobId").cloned())
            .and_then(|value| value.as_str().map(str::to_string)),
        Some("job-1".to_string())
    );
}

#[test]
fn schedule_create_schema_enforces_required_objects() {
    let registry = FunctionRegistry::new(vec![function_ids::SCHEDULE_CREATE.to_string()]);
    let err = registry
        .invoke(&direclaw::orchestration::function_registry::FunctionCall {
            function_id: function_ids::SCHEDULE_CREATE.to_string(),
            args: serde_json::Map::from_iter([
                (
                    "orchestratorId".to_string(),
                    Value::String("eng".to_string()),
                ),
                (
                    "scheduleType".to_string(),
                    Value::String("interval".to_string()),
                ),
                (
                    "schedule".to_string(),
                    Value::String("not-object".to_string()),
                ),
                (
                    "targetAction".to_string(),
                    Value::Object(serde_json::Map::new()),
                ),
            ]),
        })
        .expect_err("type mismatch should fail");

    assert!(err.to_string().contains("schedule.create.schedule"));
}
