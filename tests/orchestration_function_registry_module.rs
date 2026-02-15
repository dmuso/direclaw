use direclaw::orchestration::function_registry::{FunctionCall, FunctionRegistry};
use serde_json::{Map, Value};

#[test]
fn function_registry_module_exposes_validation_and_catalog() {
    let registry = FunctionRegistry::new(vec![
        "workflow.status".to_string(),
        "orchestrator.list".to_string(),
    ]);

    let ids = registry.available_function_ids();
    assert!(ids.iter().any(|id| id == "workflow.status"));
    assert!(ids.iter().any(|id| id == "orchestrator.list"));

    let err = registry
        .invoke(&FunctionCall {
            function_id: "workflow.status".to_string(),
            args: Map::from_iter([("runId".to_string(), Value::Bool(true))]),
        })
        .expect_err("invalid type should fail");
    assert!(err.to_string().contains("invalid argument type"));
}
