use direclaw::app::command_catalog::function_ids;
use direclaw::app::command_dispatch::{
    execute_function_invocation_with_executor, FunctionExecutionContext,
};
use direclaw::config::{AuthSyncConfig, Monitoring, Settings, SettingsOrchestrator};
use direclaw::memory::MemoryConfig;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use tempfile::tempdir;

#[test]
fn command_dispatch_module_executes_internal_orchestrator_list_function() {
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
        memory: MemoryConfig::default(),
    };

    let value = execute_function_invocation_with_executor(
        function_ids::ORCHESTRATOR_LIST,
        &Map::new(),
        FunctionExecutionContext {
            run_store: None,
            settings: Some(&settings),
        },
        |_| panic!("internal function should not invoke CLI executor"),
    )
    .expect("internal function result");

    let orchestrators = value
        .get("orchestrators")
        .and_then(|v| v.as_array())
        .expect("orchestrators array");
    assert_eq!(orchestrators, &vec![Value::String("main".to_string())]);
}
