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
use crate::config::{load_orchestrator_config, Settings};
use crate::orchestrator::{OrchestratorError, RunState, WorkflowRunStore};
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
pub use crate::app::command_dispatch::FunctionExecutionPlan;
pub use crate::app::command_dispatch::InternalFunction;
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

#[derive(Debug, Clone, Copy)]
pub struct FunctionExecutionContext<'a> {
    pub run_store: Option<&'a WorkflowRunStore>,
    pub settings: Option<&'a Settings>,
}

pub fn execute_function_invocation(
    function_id: &str,
    args: &Map<String, Value>,
    context: FunctionExecutionContext<'_>,
) -> Result<Value, OrchestratorError> {
    match plan_function_invocation(function_id, args)
        .map_err(OrchestratorError::SelectorValidation)?
    {
        FunctionExecutionPlan::CliArgs(cli_args) => execute_cli_plan(cli_args),
        FunctionExecutionPlan::Internal(internal) => execute_internal_function(internal, context),
    }
}

fn execute_cli_plan(cli_args: Vec<String>) -> Result<Value, OrchestratorError> {
    let command = cli_args.join(" ");
    let output = run_cli(cli_args).map_err(OrchestratorError::SelectorValidation)?;
    Ok(Value::Object(Map::from_iter([
        ("command".to_string(), Value::String(command)),
        ("output".to_string(), Value::String(output)),
    ])))
}

pub fn execute_internal_function(
    command: InternalFunction,
    context: FunctionExecutionContext<'_>,
) -> Result<Value, OrchestratorError> {
    match command {
        InternalFunction::WorkflowList { orchestrator_id } => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow.list requires settings context".to_string(),
                )
            })?;
            let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
            Ok(Value::Object(Map::from_iter([
                ("orchestratorId".to_string(), Value::String(orchestrator_id)),
                (
                    "workflows".to_string(),
                    Value::Array(
                        orchestrator
                            .workflows
                            .iter()
                            .map(|workflow| Value::String(workflow.id.clone()))
                            .collect(),
                    ),
                ),
            ])))
        }
        InternalFunction::WorkflowShow {
            orchestrator_id,
            workflow_id,
        } => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow.show requires settings context".to_string(),
                )
            })?;
            let orchestrator = load_orchestrator_config(settings, &orchestrator_id)?;
            let workflow = orchestrator
                .workflows
                .iter()
                .find(|workflow| workflow.id == workflow_id)
                .ok_or_else(|| {
                    OrchestratorError::SelectorValidation(format!(
                        "workflow `{workflow_id}` not found in orchestrator `{orchestrator_id}`"
                    ))
                })?;
            Ok(Value::Object(Map::from_iter([
                ("orchestratorId".to_string(), Value::String(orchestrator_id)),
                ("workflowId".to_string(), Value::String(workflow_id)),
                (
                    "workflow".to_string(),
                    serde_json::to_value(workflow)
                        .map_err(|error| OrchestratorError::SelectorJson(error.to_string()))?,
                ),
            ])))
        }
        InternalFunction::WorkflowStatus { run_id }
        | InternalFunction::WorkflowProgress { run_id } => {
            let run_store = context.run_store.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow.status/progress requires workflow run store".to_string(),
                )
            })?;
            let progress = run_store
                .load_progress(&run_id)
                .map_err(|error| remap_missing_run_error(&run_id, error))?;
            Ok(Value::Object(Map::from_iter([
                ("runId".to_string(), Value::String(run_id)),
                (
                    "progress".to_string(),
                    serde_json::to_value(progress)
                        .map_err(|error| OrchestratorError::SelectorJson(error.to_string()))?,
                ),
            ])))
        }
        InternalFunction::WorkflowCancel { run_id } => {
            let run_store = context.run_store.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "workflow.cancel requires workflow run store".to_string(),
                )
            })?;
            let mut run = run_store
                .load_run(&run_id)
                .map_err(|error| remap_missing_run_error(&run_id, error))?;
            if !run.state.clone().is_terminal() {
                let now = run.updated_at.saturating_add(1);
                run_store.transition_state(
                    &mut run,
                    RunState::Canceled,
                    now,
                    "canceled by command",
                    false,
                    "none",
                )?;
            }
            Ok(Value::Object(Map::from_iter([
                ("runId".to_string(), Value::String(run_id)),
                ("state".to_string(), Value::String(run.state.to_string())),
            ])))
        }
        InternalFunction::OrchestratorList => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "orchestrator.list requires settings context".to_string(),
                )
            })?;
            Ok(Value::Object(Map::from_iter([(
                "orchestrators".to_string(),
                Value::Array(
                    settings
                        .orchestrators
                        .keys()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            )])))
        }
        InternalFunction::OrchestratorShow { orchestrator_id } => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "orchestrator.show requires settings context".to_string(),
                )
            })?;
            let entry = settings
                .orchestrators
                .get(&orchestrator_id)
                .ok_or_else(|| {
                    OrchestratorError::SelectorValidation(format!(
                        "unknown orchestrator `{orchestrator_id}`"
                    ))
                })?;
            let private_workspace = settings.resolve_private_workspace(&orchestrator_id)?;
            Ok(Value::Object(Map::from_iter([
                ("orchestratorId".to_string(), Value::String(orchestrator_id)),
                (
                    "privateWorkspace".to_string(),
                    Value::String(private_workspace.display().to_string()),
                ),
                (
                    "sharedAccess".to_string(),
                    Value::Array(
                        entry
                            .shared_access
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                ),
            ])))
        }
        InternalFunction::ChannelProfileList => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "channel_profile.list requires settings context".to_string(),
                )
            })?;
            Ok(Value::Object(Map::from_iter([(
                "channelProfiles".to_string(),
                Value::Array(
                    settings
                        .channel_profiles
                        .keys()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            )])))
        }
        InternalFunction::ChannelProfileShow { channel_profile_id } => {
            let settings = context.settings.ok_or_else(|| {
                OrchestratorError::SelectorValidation(
                    "channel_profile.show requires settings context".to_string(),
                )
            })?;
            let profile = settings
                .channel_profiles
                .get(&channel_profile_id)
                .ok_or_else(|| {
                    OrchestratorError::SelectorValidation(format!(
                        "unknown channel profile `{channel_profile_id}`"
                    ))
                })?;
            Ok(Value::Object(Map::from_iter([
                (
                    "channelProfileId".to_string(),
                    Value::String(channel_profile_id),
                ),
                (
                    "channel".to_string(),
                    Value::String(profile.channel.to_string()),
                ),
                (
                    "orchestratorId".to_string(),
                    Value::String(profile.orchestrator_id.clone()),
                ),
                (
                    "slackAppUserId".to_string(),
                    profile
                        .slack_app_user_id
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                ),
                (
                    "requireMentionInChannels".to_string(),
                    profile
                        .require_mention_in_channels
                        .map(Value::Bool)
                        .unwrap_or(Value::Null),
                ),
            ])))
        }
    }
}

fn remap_missing_run_error(run_id: &str, err: OrchestratorError) -> OrchestratorError {
    match err {
        OrchestratorError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            OrchestratorError::UnknownRunId {
                run_id: run_id.to_string(),
            }
        }
        _ => err,
    }
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
    use crate::config::{AuthSyncConfig, Monitoring, SettingsOrchestrator};
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
