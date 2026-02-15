use crate::app::command_catalog::function_ids;
use crate::config::{load_orchestrator_config, Settings};
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::run_store::{RunState, WorkflowRunStore};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalFunction {
    WorkflowList {
        orchestrator_id: String,
    },
    WorkflowShow {
        orchestrator_id: String,
        workflow_id: String,
    },
    WorkflowStatus {
        run_id: String,
    },
    WorkflowProgress {
        run_id: String,
    },
    WorkflowCancel {
        run_id: String,
    },
    OrchestratorList,
    OrchestratorShow {
        orchestrator_id: String,
    },
    ChannelProfileList,
    ChannelProfileShow {
        channel_profile_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionExecutionPlan {
    CliArgs(Vec<String>),
    Internal(InternalFunction),
}

#[derive(Debug, Clone, Copy)]
pub struct FunctionExecutionContext<'a> {
    pub run_store: Option<&'a WorkflowRunStore>,
    pub settings: Option<&'a Settings>,
}

pub fn execute_function_invocation_with_executor<F>(
    function_id: &str,
    args: &Map<String, Value>,
    context: FunctionExecutionContext<'_>,
    cli_executor: F,
) -> Result<Value, OrchestratorError>
where
    F: Fn(Vec<String>) -> Result<String, String>,
{
    match plan_function_invocation(function_id, args)
        .map_err(OrchestratorError::SelectorValidation)?
    {
        FunctionExecutionPlan::CliArgs(cli_args) => execute_cli_plan(cli_args, cli_executor),
        FunctionExecutionPlan::Internal(internal) => execute_internal_function(internal, context),
    }
}

fn execute_cli_plan<F>(cli_args: Vec<String>, cli_executor: F) -> Result<Value, OrchestratorError>
where
    F: Fn(Vec<String>) -> Result<String, String>,
{
    let command = cli_args.join(" ");
    let output = cli_executor(cli_args).map_err(OrchestratorError::SelectorValidation)?;
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

pub fn plan_function_invocation(
    function_id: &str,
    args: &Map<String, Value>,
) -> Result<FunctionExecutionPlan, String> {
    match function_id {
        function_ids::DAEMON_START => Ok(FunctionExecutionPlan::CliArgs(vec!["start".to_string()])),
        function_ids::DAEMON_STOP => Ok(FunctionExecutionPlan::CliArgs(vec!["stop".to_string()])),
        function_ids::DAEMON_RESTART => {
            Ok(FunctionExecutionPlan::CliArgs(vec!["restart".to_string()]))
        }
        function_ids::DAEMON_STATUS => {
            Ok(FunctionExecutionPlan::CliArgs(vec!["status".to_string()]))
        }
        function_ids::DAEMON_LOGS => Ok(FunctionExecutionPlan::CliArgs(vec!["logs".to_string()])),
        function_ids::DAEMON_SETUP => Ok(FunctionExecutionPlan::CliArgs(vec!["setup".to_string()])),
        function_ids::DAEMON_SEND => {
            let profile_id = required_string_arg(args, "channelProfileId")?;
            let message = required_string_arg(args, "message")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "send".to_string(),
                profile_id,
                message,
            ]))
        }
        function_ids::CHANNELS_RESET => Ok(FunctionExecutionPlan::CliArgs(vec![
            "channels".to_string(),
            "reset".to_string(),
        ])),
        function_ids::CHANNELS_SLACK_SYNC => Ok(FunctionExecutionPlan::CliArgs(vec![
            "channels".to_string(),
            "slack".to_string(),
            "sync".to_string(),
        ])),
        function_ids::PROVIDER_SHOW => {
            Ok(FunctionExecutionPlan::CliArgs(vec!["provider".to_string()]))
        }
        function_ids::PROVIDER_SET => {
            let provider = required_string_arg(args, "provider")?;
            let mut cli_args = vec!["provider".to_string(), provider];
            if let Some(model) = optional_string_arg(args, "model")? {
                cli_args.push("--model".to_string());
                cli_args.push(model);
            }
            Ok(FunctionExecutionPlan::CliArgs(cli_args))
        }
        function_ids::MODEL_SHOW => Ok(FunctionExecutionPlan::CliArgs(vec!["model".to_string()])),
        function_ids::MODEL_SET => {
            let model = required_string_arg(args, "model")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "model".to_string(),
                model,
            ]))
        }
        function_ids::AGENT_LIST => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "agent".to_string(),
                "list".to_string(),
                orchestrator_id,
            ]))
        }
        function_ids::AGENT_ADD => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let agent_id = required_string_arg(args, "agentId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "agent".to_string(),
                "add".to_string(),
                orchestrator_id,
                agent_id,
            ]))
        }
        function_ids::AGENT_SHOW => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let agent_id = required_string_arg(args, "agentId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "agent".to_string(),
                "show".to_string(),
                orchestrator_id,
                agent_id,
            ]))
        }
        function_ids::AGENT_REMOVE => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let agent_id = required_string_arg(args, "agentId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "agent".to_string(),
                "remove".to_string(),
                orchestrator_id,
                agent_id,
            ]))
        }
        function_ids::AGENT_RESET => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let agent_id = required_string_arg(args, "agentId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "agent".to_string(),
                "reset".to_string(),
                orchestrator_id,
                agent_id,
            ]))
        }
        function_ids::ORCHESTRATOR_ADD => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "orchestrator".to_string(),
                "add".to_string(),
                orchestrator_id,
            ]))
        }
        function_ids::WORKFLOW_LIST => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            Ok(FunctionExecutionPlan::Internal(
                InternalFunction::WorkflowList { orchestrator_id },
            ))
        }
        function_ids::WORKFLOW_SHOW => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let workflow_id = required_string_arg(args, "workflowId")?;
            Ok(FunctionExecutionPlan::Internal(
                InternalFunction::WorkflowShow {
                    orchestrator_id,
                    workflow_id,
                },
            ))
        }
        function_ids::WORKFLOW_ADD => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let workflow_id = required_string_arg(args, "workflowId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "workflow".to_string(),
                "add".to_string(),
                orchestrator_id,
                workflow_id,
            ]))
        }
        function_ids::WORKFLOW_REMOVE => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let workflow_id = required_string_arg(args, "workflowId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "workflow".to_string(),
                "remove".to_string(),
                orchestrator_id,
                workflow_id,
            ]))
        }
        function_ids::WORKFLOW_RUN => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let workflow_id = required_string_arg(args, "workflowId")?;
            let mut cli_args = vec![
                "workflow".to_string(),
                "run".to_string(),
                orchestrator_id,
                workflow_id,
            ];
            if let Some(inputs) = optional_object_arg(args, "inputs") {
                for (key, value) in inputs {
                    let encoded = value
                        .as_str()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| value.to_string());
                    cli_args.push("--input".to_string());
                    cli_args.push(format!("{key}={encoded}"));
                }
            }
            Ok(FunctionExecutionPlan::CliArgs(cli_args))
        }
        function_ids::WORKFLOW_STATUS => {
            let run_id = required_string_arg(args, "runId")?;
            Ok(FunctionExecutionPlan::Internal(
                InternalFunction::WorkflowStatus { run_id },
            ))
        }
        function_ids::WORKFLOW_PROGRESS => {
            let run_id = required_string_arg(args, "runId")?;
            Ok(FunctionExecutionPlan::Internal(
                InternalFunction::WorkflowProgress { run_id },
            ))
        }
        function_ids::WORKFLOW_CANCEL => {
            let run_id = required_string_arg(args, "runId")?;
            Ok(FunctionExecutionPlan::Internal(
                InternalFunction::WorkflowCancel { run_id },
            ))
        }
        function_ids::ORCHESTRATOR_LIST => Ok(FunctionExecutionPlan::Internal(
            InternalFunction::OrchestratorList,
        )),
        function_ids::ORCHESTRATOR_SHOW => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            Ok(FunctionExecutionPlan::Internal(
                InternalFunction::OrchestratorShow { orchestrator_id },
            ))
        }
        function_ids::ORCHESTRATOR_REMOVE => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "orchestrator".to_string(),
                "remove".to_string(),
                orchestrator_id,
            ]))
        }
        function_ids::ORCHESTRATOR_SET_PRIVATE_WORKSPACE => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let path = required_string_arg(args, "path")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "orchestrator".to_string(),
                "set-private-workspace".to_string(),
                orchestrator_id,
                path,
            ]))
        }
        function_ids::ORCHESTRATOR_GRANT_SHARED_ACCESS => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let shared_key = required_string_arg(args, "sharedKey")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "orchestrator".to_string(),
                "grant-shared-access".to_string(),
                orchestrator_id,
                shared_key,
            ]))
        }
        function_ids::ORCHESTRATOR_REVOKE_SHARED_ACCESS => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let shared_key = required_string_arg(args, "sharedKey")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "orchestrator".to_string(),
                "revoke-shared-access".to_string(),
                orchestrator_id,
                shared_key,
            ]))
        }
        function_ids::ORCHESTRATOR_SET_SELECTOR_AGENT => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let agent_id = required_string_arg(args, "agentId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "orchestrator".to_string(),
                "set-selector-agent".to_string(),
                orchestrator_id,
                agent_id,
            ]))
        }
        function_ids::ORCHESTRATOR_SET_DEFAULT_WORKFLOW => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let workflow_id = required_string_arg(args, "workflowId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "orchestrator".to_string(),
                "set-default-workflow".to_string(),
                orchestrator_id,
                workflow_id,
            ]))
        }
        function_ids::ORCHESTRATOR_SET_SELECTION_MAX_RETRIES => {
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let count = required_u32_arg(args, "count")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "orchestrator".to_string(),
                "set-selection-max-retries".to_string(),
                orchestrator_id,
                count.to_string(),
            ]))
        }
        function_ids::CHANNEL_PROFILE_LIST => Ok(FunctionExecutionPlan::Internal(
            InternalFunction::ChannelProfileList,
        )),
        function_ids::CHANNEL_PROFILE_SHOW => {
            let channel_profile_id = required_string_arg(args, "channelProfileId")?;
            Ok(FunctionExecutionPlan::Internal(
                InternalFunction::ChannelProfileShow { channel_profile_id },
            ))
        }
        function_ids::CHANNEL_PROFILE_ADD => {
            let profile_id = required_string_arg(args, "channelProfileId")?;
            let channel = required_string_arg(args, "channel")?;
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            let mut cli_args = vec![
                "channel-profile".to_string(),
                "add".to_string(),
                profile_id,
                channel,
                orchestrator_id,
            ];
            if let Some(user_id) = optional_string_arg(args, "slackAppUserId")? {
                cli_args.push("--slack-app-user-id".to_string());
                cli_args.push(user_id);
            }
            if let Some(require_mention) = optional_bool_arg(args, "requireMentionInChannels")? {
                cli_args.push("--require-mention-in-channels".to_string());
                cli_args.push(require_mention.to_string());
            }
            Ok(FunctionExecutionPlan::CliArgs(cli_args))
        }
        function_ids::CHANNEL_PROFILE_REMOVE => {
            let profile_id = required_string_arg(args, "channelProfileId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "channel-profile".to_string(),
                "remove".to_string(),
                profile_id,
            ]))
        }
        function_ids::CHANNEL_PROFILE_SET_ORCHESTRATOR => {
            let profile_id = required_string_arg(args, "channelProfileId")?;
            let orchestrator_id = required_string_arg(args, "orchestratorId")?;
            Ok(FunctionExecutionPlan::CliArgs(vec![
                "channel-profile".to_string(),
                "set-orchestrator".to_string(),
                profile_id,
                orchestrator_id,
            ]))
        }
        function_ids::UPDATE_CHECK => Ok(FunctionExecutionPlan::CliArgs(vec![
            "update".to_string(),
            "check".to_string(),
        ])),
        function_ids::UPDATE_APPLY => Ok(FunctionExecutionPlan::CliArgs(vec![
            "update".to_string(),
            "apply".to_string(),
        ])),
        function_ids::DAEMON_ATTACH => {
            Ok(FunctionExecutionPlan::CliArgs(vec!["attach".to_string()]))
        }
        _ => Err(format!("unknown function id `{function_id}`")),
    }
}

fn required_string_arg(args: &Map<String, Value>, arg: &str) -> Result<String, String> {
    match args.get(arg) {
        Some(Value::String(v)) if !v.trim().is_empty() => Ok(v.clone()),
        Some(Value::String(_)) => Err(format!("missing required function argument `{arg}`")),
        Some(_) => Err(format!("argument `{arg}` must be a string")),
        None => Err(format!("missing required function argument `{arg}`")),
    }
}

fn optional_string_arg(args: &Map<String, Value>, arg: &str) -> Result<Option<String>, String> {
    match args.get(arg) {
        Some(Value::String(v)) if !v.trim().is_empty() => Ok(Some(v.clone())),
        Some(Value::String(_)) => Err(format!("missing required function argument `{arg}`")),
        Some(_) => Err(format!("argument `{arg}` must be a string")),
        None => Ok(None),
    }
}

fn required_u32_arg(args: &Map<String, Value>, arg: &str) -> Result<u32, String> {
    match args.get(arg) {
        Some(value) => {
            if let Some(v) = value.as_u64() {
                return u32::try_from(v).map_err(|_| format!("argument `{arg}` is too large"));
            }
            if let Some(v) = value.as_i64() {
                return u32::try_from(v).map_err(|_| format!("argument `{arg}` must be >= 0"));
            }
            Err(format!("argument `{arg}` must be an integer"))
        }
        None => Err(format!("missing required function argument `{arg}`")),
    }
}

fn optional_bool_arg(args: &Map<String, Value>, arg: &str) -> Result<Option<bool>, String> {
    match args.get(arg) {
        Some(Value::Bool(v)) => Ok(Some(*v)),
        Some(_) => Err(format!("argument `{arg}` must be a boolean")),
        None => Ok(None),
    }
}

fn optional_object_arg<'a>(
    args: &'a Map<String, Value>,
    arg: &str,
) -> Option<&'a Map<String, Value>> {
    args.get(arg).and_then(|value| value.as_object())
}
