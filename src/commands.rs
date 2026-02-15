pub use crate::app::cli::cli_help_lines;
pub use crate::app::cli::parse_cli_verb;
pub use crate::app::cli::selector_help_lines;
pub use crate::app::cli::CliVerb;
pub use crate::app::command_catalog::function_ids;
pub use crate::app::command_catalog::FunctionArgDef;
pub use crate::app::command_catalog::FunctionArgTypeDef;
pub use crate::app::command_catalog::FunctionDef;
pub use crate::app::command_catalog::V1_FUNCTIONS;
pub use crate::app::command_dispatch::plan_function_invocation;
pub use crate::app::command_dispatch::FunctionExecutionContext;
pub use crate::app::command_dispatch::FunctionExecutionPlan;
pub use crate::app::command_dispatch::InternalFunction;
pub use crate::app::command_dispatch::{
    execute_function_invocation_with_executor, execute_internal_function,
};
pub use crate::app::command_handlers::execute_function_invocation;
pub use crate::app::command_handlers::run_cli;
pub use crate::app::command_support::default_orchestrator_config;
pub use crate::app::command_support::ensure_runtime_root;
pub use crate::app::command_support::load_orchestrator_or_err;
pub use crate::app::command_support::load_preferences;
pub use crate::app::command_support::load_settings;
pub use crate::app::command_support::map_config_err;
pub use crate::app::command_support::now_nanos;
pub use crate::app::command_support::now_secs;
pub use crate::app::command_support::remove_orchestrator_config;
pub use crate::app::command_support::save_orchestrator_config;
pub use crate::app::command_support::save_orchestrator_registry;
pub use crate::app::command_support::save_preferences;
pub use crate::app::command_support::save_settings;
pub use crate::app::command_support::RuntimePreferences;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthSyncConfig, Monitoring, Settings, SettingsOrchestrator};
    use serde_json::{Map, Value};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    #[test]
    fn shared_executor_routes_cli_style_function_calls() {
        let err = execute_function_invocation(
            function_ids::UPDATE_APPLY,
            &Map::new(),
            FunctionExecutionContext {
                run_store: None,
                settings: None,
            },
        )
        .expect_err("update.apply should remain blocked");

        assert!(err.to_string().contains("update apply is unsupported"));
    }

    #[test]
    fn shared_executor_handles_internal_functions() {
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
}
