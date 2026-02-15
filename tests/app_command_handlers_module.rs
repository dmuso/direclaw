use direclaw::app::command_catalog::function_ids;
use direclaw::app::command_dispatch::FunctionExecutionContext;
use direclaw::app::command_handlers::{execute_function_invocation, run_cli};
use direclaw::config::{AuthSyncConfig, Monitoring, Settings, SettingsOrchestrator};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use tempfile::tempdir;

#[test]
fn app_command_handlers_run_cli_supports_unknown_command_error() {
    let args = vec!["unknown-command".to_string()];
    let err = run_cli(args).expect_err("unknown command");
    assert!(err.contains("unknown command"));
}

#[test]
fn app_command_handlers_execute_function_invocation_routes_internal_functions() {
    let temp = tempdir().expect("tempdir");
    let settings = Settings {
        workspaces_path: temp.path().to_path_buf(),
        shared_workspaces: BTreeMap::new(),
        orchestrators: BTreeMap::from_iter([(
            "main".to_string(),
            SettingsOrchestrator {
                private_workspace: None,
                shared_access: Vec::new(),
            },
        )]),
        channel_profiles: BTreeMap::new(),
        monitoring: Monitoring::default(),
        channels: BTreeMap::new(),
        auth_sync: AuthSyncConfig::default(),
    };

    let value = execute_function_invocation(
        function_ids::ORCHESTRATOR_LIST,
        &Map::new(),
        FunctionExecutionContext {
            run_store: None,
            settings: Some(&settings),
        },
    )
    .expect("internal function result");

    let orchestrators = value
        .get("orchestrators")
        .and_then(|v| v.as_array())
        .expect("orchestrators array");
    assert_eq!(orchestrators, &vec![Value::String("main".to_string())]);
}
