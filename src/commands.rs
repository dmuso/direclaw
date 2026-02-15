use crate::app::command_handlers::agents::cmd_orchestrator_agent;
use crate::app::command_handlers::attach::cmd_attach;
use crate::app::command_handlers::auth::cmd_auth;
use crate::app::command_handlers::channel_profiles::cmd_channel_profile;
use crate::app::command_handlers::channels::{cmd_channels, cmd_send};
use crate::app::command_handlers::daemon::{
    cmd_logs, cmd_restart, cmd_start, cmd_status, cmd_stop, cmd_supervisor,
};
use crate::app::command_handlers::doctor::cmd_doctor;
use crate::app::command_handlers::orchestrators::cmd_orchestrator;
use crate::app::command_handlers::provider::{cmd_model, cmd_provider};
use crate::app::command_handlers::update::cmd_update;
use crate::app::command_handlers::workflows::cmd_workflow;
use crate::orchestrator::OrchestratorError;
use serde_json::{Map, Value};

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

pub fn execute_function_invocation(
    function_id: &str,
    args: &Map<String, Value>,
    context: FunctionExecutionContext<'_>,
) -> Result<Value, OrchestratorError> {
    execute_function_invocation_with_executor(function_id, args, context, run_cli)
}

pub fn run_cli(args: Vec<String>) -> Result<String, String> {
    if args.is_empty() {
        return Ok(crate::app::cli::help_text());
    }

    match parse_cli_verb(args[0].as_str()) {
        CliVerb::Setup => crate::tui::setup::cmd_setup(),
        CliVerb::Start => cmd_start(),
        CliVerb::Stop => cmd_stop(),
        CliVerb::Restart => cmd_restart(),
        CliVerb::Status => cmd_status(),
        CliVerb::Logs => cmd_logs(),
        CliVerb::Send => cmd_send(&args[1..]),
        CliVerb::Update => cmd_update(&args[1..]),
        CliVerb::Doctor => cmd_doctor(),
        CliVerb::Attach => cmd_attach(),
        CliVerb::Channels => cmd_channels(&args[1..]),
        CliVerb::Provider => cmd_provider(&args[1..]),
        CliVerb::Model => cmd_model(&args[1..]),
        CliVerb::Agent => cmd_orchestrator_agent(&args[1..]),
        CliVerb::Orchestrator => cmd_orchestrator(&args[1..]),
        CliVerb::OrchestratorAgent => cmd_orchestrator_agent(&args[1..]),
        CliVerb::Workflow => cmd_workflow(&args[1..]),
        CliVerb::ChannelProfile => cmd_channel_profile(&args[1..]),
        CliVerb::Auth => cmd_auth(&args[1..]),
        CliVerb::Supervisor => cmd_supervisor(&args[1..]),
        CliVerb::Unknown => Err(format!("unknown command `{}`", args[0])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthSyncConfig, Monitoring, Settings, SettingsOrchestrator};
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
