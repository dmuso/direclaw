#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionArgTypeDef {
    String,
    Boolean,
    Integer,
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionArgDef {
    pub name: &'static str,
    pub arg_type: FunctionArgTypeDef,
    pub required: bool,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionDef {
    pub function_id: &'static str,
    pub description: &'static str,
    pub args: &'static [FunctionArgDef],
    pub read_only: bool,
}

pub mod function_ids {
    pub const DAEMON_START: &str = "daemon.start";
    pub const DAEMON_STOP: &str = "daemon.stop";
    pub const DAEMON_RESTART: &str = "daemon.restart";
    pub const DAEMON_STATUS: &str = "daemon.status";
    pub const DAEMON_LOGS: &str = "daemon.logs";
    pub const DAEMON_SETUP: &str = "daemon.setup";
    pub const DAEMON_SEND: &str = "daemon.send";
    pub const CHANNELS_RESET: &str = "channels.reset";
    pub const CHANNELS_SLACK_SYNC: &str = "channels.slack_sync";
    pub const PROVIDER_SHOW: &str = "provider.show";
    pub const PROVIDER_SET: &str = "provider.set";
    pub const MODEL_SHOW: &str = "model.show";
    pub const MODEL_SET: &str = "model.set";
    pub const AGENT_LIST: &str = "agent.list";
    pub const AGENT_ADD: &str = "agent.add";
    pub const AGENT_SHOW: &str = "agent.show";
    pub const AGENT_REMOVE: &str = "agent.remove";
    pub const AGENT_RESET: &str = "agent.reset";
    pub const ORCHESTRATOR_LIST: &str = "orchestrator.list";
    pub const ORCHESTRATOR_ADD: &str = "orchestrator.add";
    pub const ORCHESTRATOR_SHOW: &str = "orchestrator.show";
    pub const ORCHESTRATOR_REMOVE: &str = "orchestrator.remove";
    pub const ORCHESTRATOR_SET_PRIVATE_WORKSPACE: &str = "orchestrator.set_private_workspace";
    pub const ORCHESTRATOR_GRANT_SHARED_ACCESS: &str = "orchestrator.grant_shared_access";
    pub const ORCHESTRATOR_REVOKE_SHARED_ACCESS: &str = "orchestrator.revoke_shared_access";
    pub const ORCHESTRATOR_SET_SELECTOR_AGENT: &str = "orchestrator.set_selector_agent";
    pub const ORCHESTRATOR_SET_DEFAULT_WORKFLOW: &str = "orchestrator.set_default_workflow";
    pub const ORCHESTRATOR_SET_SELECTION_MAX_RETRIES: &str =
        "orchestrator.set_selection_max_retries";
    pub const WORKFLOW_LIST: &str = "workflow.list";
    pub const WORKFLOW_SHOW: &str = "workflow.show";
    pub const WORKFLOW_ADD: &str = "workflow.add";
    pub const WORKFLOW_REMOVE: &str = "workflow.remove";
    pub const WORKFLOW_RUN: &str = "workflow.run";
    pub const WORKFLOW_STATUS: &str = "workflow.status";
    pub const WORKFLOW_PROGRESS: &str = "workflow.progress";
    pub const WORKFLOW_CANCEL: &str = "workflow.cancel";
    pub const CHANNEL_PROFILE_LIST: &str = "channel_profile.list";
    pub const CHANNEL_PROFILE_ADD: &str = "channel_profile.add";
    pub const CHANNEL_PROFILE_SHOW: &str = "channel_profile.show";
    pub const CHANNEL_PROFILE_REMOVE: &str = "channel_profile.remove";
    pub const CHANNEL_PROFILE_SET_ORCHESTRATOR: &str = "channel_profile.set_orchestrator";
    pub const UPDATE_CHECK: &str = "update.check";
    pub const UPDATE_APPLY: &str = "update.apply";
    pub const DAEMON_ATTACH: &str = "daemon.attach";
}

const DAEMON_SEND_ARGS: &[FunctionArgDef] = &[
    FunctionArgDef {
        name: "channelProfileId",
        arg_type: FunctionArgTypeDef::String,
        required: true,
        description: "Target channel profile id",
    },
    FunctionArgDef {
        name: "message",
        arg_type: FunctionArgTypeDef::String,
        required: true,
        description: "Message content",
    },
];

const PROVIDER_SET_ARGS: &[FunctionArgDef] = &[
    FunctionArgDef {
        name: "provider",
        arg_type: FunctionArgTypeDef::String,
        required: true,
        description: "Provider id: anthropic or openai",
    },
    FunctionArgDef {
        name: "model",
        arg_type: FunctionArgTypeDef::String,
        required: false,
        description: "Optional model identifier",
    },
];

const MODEL_SET_ARGS: &[FunctionArgDef] = &[FunctionArgDef {
    name: "model",
    arg_type: FunctionArgTypeDef::String,
    required: true,
    description: "Model identifier",
}];

const ORCHESTRATOR_ID_ARG: FunctionArgDef = FunctionArgDef {
    name: "orchestratorId",
    arg_type: FunctionArgTypeDef::String,
    required: true,
    description: "Target orchestrator id",
};

const AGENT_ID_ARG: FunctionArgDef = FunctionArgDef {
    name: "agentId",
    arg_type: FunctionArgTypeDef::String,
    required: true,
    description: "Agent id",
};

const WORKFLOW_ID_ARG: FunctionArgDef = FunctionArgDef {
    name: "workflowId",
    arg_type: FunctionArgTypeDef::String,
    required: true,
    description: "Workflow id",
};

const RUN_ID_ARG: FunctionArgDef = FunctionArgDef {
    name: "runId",
    arg_type: FunctionArgTypeDef::String,
    required: true,
    description: "Workflow run id",
};

const PROFILE_ID_ARG: FunctionArgDef = FunctionArgDef {
    name: "channelProfileId",
    arg_type: FunctionArgTypeDef::String,
    required: true,
    description: "Channel profile id",
};

const AGENT_ORCHESTRATOR_ARGS: &[FunctionArgDef] = &[ORCHESTRATOR_ID_ARG, AGENT_ID_ARG];
const WORKFLOW_ORCHESTRATOR_ARGS: &[FunctionArgDef] = &[ORCHESTRATOR_ID_ARG, WORKFLOW_ID_ARG];

pub const V1_FUNCTIONS: &[FunctionDef] = &[
    FunctionDef {
        function_id: function_ids::DAEMON_START,
        description: "Start runtime workers",
        args: &[],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::DAEMON_STOP,
        description: "Stop runtime workers",
        args: &[],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::DAEMON_RESTART,
        description: "Restart runtime workers",
        args: &[],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::DAEMON_STATUS,
        description: "Read runtime status and worker health",
        args: &[],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::DAEMON_LOGS,
        description: "Read recent runtime logs",
        args: &[],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::DAEMON_SETUP,
        description: "Create default config and state root",
        args: &[],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::DAEMON_SEND,
        description: "Send message to channel profile",
        args: DAEMON_SEND_ARGS,
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::CHANNELS_RESET,
        description: "Reset channel state directories",
        args: &[],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::CHANNELS_SLACK_SYNC,
        description: "Run one Slack sync pass",
        args: &[],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::PROVIDER_SHOW,
        description: "Show current provider/model preferences",
        args: &[],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::PROVIDER_SET,
        description: "Set provider preference and optional model",
        args: PROVIDER_SET_ARGS,
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::MODEL_SHOW,
        description: "Show current model preference",
        args: &[],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::MODEL_SET,
        description: "Set model preference",
        args: MODEL_SET_ARGS,
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::AGENT_LIST,
        description: "List orchestrator agent ids",
        args: &[ORCHESTRATOR_ID_ARG],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::AGENT_ADD,
        description: "Add orchestrator-local agent",
        args: AGENT_ORCHESTRATOR_ARGS,
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::AGENT_SHOW,
        description: "Show orchestrator-local agent",
        args: AGENT_ORCHESTRATOR_ARGS,
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::AGENT_REMOVE,
        description: "Remove orchestrator-local agent",
        args: AGENT_ORCHESTRATOR_ARGS,
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::AGENT_RESET,
        description: "Reset orchestrator-local agent defaults",
        args: AGENT_ORCHESTRATOR_ARGS,
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_LIST,
        description: "List orchestrator ids",
        args: &[],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_ADD,
        description: "Add orchestrator and bootstrap config",
        args: &[FunctionArgDef {
            name: "orchestratorId",
            arg_type: FunctionArgTypeDef::String,
            required: true,
            description: "Orchestrator id",
        }],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_SHOW,
        description: "Show one orchestrator configuration summary",
        args: &[ORCHESTRATOR_ID_ARG],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_REMOVE,
        description: "Remove orchestrator from settings",
        args: &[ORCHESTRATOR_ID_ARG],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_SET_PRIVATE_WORKSPACE,
        description: "Set orchestrator private workspace path",
        args: &[
            ORCHESTRATOR_ID_ARG,
            FunctionArgDef {
                name: "path",
                arg_type: FunctionArgTypeDef::String,
                required: true,
                description: "Absolute private workspace path",
            },
        ],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_GRANT_SHARED_ACCESS,
        description: "Grant shared workspace key to orchestrator",
        args: &[
            ORCHESTRATOR_ID_ARG,
            FunctionArgDef {
                name: "sharedKey",
                arg_type: FunctionArgTypeDef::String,
                required: true,
                description: "Shared workspace key",
            },
        ],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_REVOKE_SHARED_ACCESS,
        description: "Revoke shared workspace key from orchestrator",
        args: &[
            ORCHESTRATOR_ID_ARG,
            FunctionArgDef {
                name: "sharedKey",
                arg_type: FunctionArgTypeDef::String,
                required: true,
                description: "Shared workspace key",
            },
        ],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_SET_SELECTOR_AGENT,
        description: "Set orchestrator selector agent id",
        args: &[
            ORCHESTRATOR_ID_ARG,
            FunctionArgDef {
                name: "agentId",
                arg_type: FunctionArgTypeDef::String,
                required: true,
                description: "Selector agent id",
            },
        ],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_SET_DEFAULT_WORKFLOW,
        description: "Set orchestrator default workflow id",
        args: WORKFLOW_ORCHESTRATOR_ARGS,
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::ORCHESTRATOR_SET_SELECTION_MAX_RETRIES,
        description: "Set selector retry limit",
        args: &[
            ORCHESTRATOR_ID_ARG,
            FunctionArgDef {
                name: "count",
                arg_type: FunctionArgTypeDef::Integer,
                required: true,
                description: "Retry count >= 1",
            },
        ],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::WORKFLOW_LIST,
        description: "List workflows for an orchestrator",
        args: &[ORCHESTRATOR_ID_ARG],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::WORKFLOW_SHOW,
        description: "Show one workflow definition",
        args: &[
            ORCHESTRATOR_ID_ARG,
            FunctionArgDef {
                name: "workflowId",
                arg_type: FunctionArgTypeDef::String,
                required: true,
                description: "Workflow id in orchestrator scope",
            },
        ],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::WORKFLOW_ADD,
        description: "Add workflow to orchestrator config",
        args: WORKFLOW_ORCHESTRATOR_ARGS,
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::WORKFLOW_REMOVE,
        description: "Remove workflow from orchestrator config",
        args: WORKFLOW_ORCHESTRATOR_ARGS,
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::WORKFLOW_RUN,
        description: "Start a workflow run",
        args: &[
            ORCHESTRATOR_ID_ARG,
            WORKFLOW_ID_ARG,
            FunctionArgDef {
                name: "inputs",
                arg_type: FunctionArgTypeDef::Object,
                required: false,
                description: "Optional key/value workflow inputs",
            },
        ],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::WORKFLOW_STATUS,
        description: "Read workflow run status summary",
        args: &[RUN_ID_ARG],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::WORKFLOW_PROGRESS,
        description: "Read full workflow progress payload",
        args: &[RUN_ID_ARG],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::WORKFLOW_CANCEL,
        description: "Cancel a workflow run",
        args: &[RUN_ID_ARG],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::CHANNEL_PROFILE_LIST,
        description: "List configured channel profile ids",
        args: &[],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::CHANNEL_PROFILE_ADD,
        description: "Add channel profile mapping",
        args: &[
            PROFILE_ID_ARG,
            FunctionArgDef {
                name: "channel",
                arg_type: FunctionArgTypeDef::String,
                required: true,
                description: "Channel backend id",
            },
            ORCHESTRATOR_ID_ARG,
            FunctionArgDef {
                name: "slackAppUserId",
                arg_type: FunctionArgTypeDef::String,
                required: false,
                description: "Slack bot user id",
            },
            FunctionArgDef {
                name: "requireMentionInChannels",
                arg_type: FunctionArgTypeDef::Boolean,
                required: false,
                description: "Slack mention requirement in channels",
            },
        ],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::CHANNEL_PROFILE_SHOW,
        description: "Show one channel profile mapping",
        args: &[PROFILE_ID_ARG],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::CHANNEL_PROFILE_REMOVE,
        description: "Remove channel profile mapping",
        args: &[PROFILE_ID_ARG],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::CHANNEL_PROFILE_SET_ORCHESTRATOR,
        description: "Update channel profile orchestrator mapping",
        args: &[
            PROFILE_ID_ARG,
            FunctionArgDef {
                name: "orchestratorId",
                arg_type: FunctionArgTypeDef::String,
                required: true,
                description: "Mapped orchestrator id",
            },
        ],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::UPDATE_CHECK,
        description: "Check for updates",
        args: &[],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::UPDATE_APPLY,
        description: "Apply update (unsupported in this build)",
        args: &[],
        read_only: false,
    },
    FunctionDef {
        function_id: function_ids::DAEMON_ATTACH,
        description: "Attach to supervisor or return workflow summary",
        args: &[],
        read_only: true,
    },
];

pub fn selector_help_lines() -> Vec<String> {
    let mut defs: Vec<_> = V1_FUNCTIONS.iter().collect();
    defs.sort_by(|a, b| a.function_id.cmp(b.function_id));
    defs.into_iter()
        .map(|def| format!("  {0:36} {1}", def.function_id, def.description))
        .collect()
}

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
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliVerb {
    Setup,
    Start,
    Stop,
    Restart,
    Status,
    Logs,
    Send,
    Update,
    Doctor,
    Attach,
    Channels,
    Provider,
    Model,
    Agent,
    Orchestrator,
    OrchestratorAgent,
    Workflow,
    ChannelProfile,
    Auth,
    Supervisor,
    Unknown,
}

pub fn parse_cli_verb(input: &str) -> CliVerb {
    match input {
        "setup" => CliVerb::Setup,
        "start" => CliVerb::Start,
        "stop" => CliVerb::Stop,
        "restart" => CliVerb::Restart,
        "status" => CliVerb::Status,
        "logs" => CliVerb::Logs,
        "send" => CliVerb::Send,
        "update" => CliVerb::Update,
        "doctor" => CliVerb::Doctor,
        "attach" => CliVerb::Attach,
        "channels" => CliVerb::Channels,
        "provider" => CliVerb::Provider,
        "model" => CliVerb::Model,
        "agent" => CliVerb::Agent,
        "orchestrator" => CliVerb::Orchestrator,
        "orchestrator-agent" => CliVerb::OrchestratorAgent,
        "workflow" => CliVerb::Workflow,
        "channel-profile" => CliVerb::ChannelProfile,
        "auth" => CliVerb::Auth,
        "__supervisor" => CliVerb::Supervisor,
        _ => CliVerb::Unknown,
    }
}

pub fn cli_help_lines() -> Vec<String> {
    vec![
        "Commands:".to_string(),
        "  setup                                Initialize state/config/runtime directories"
            .to_string(),
        "  start                                Start the DireClaw supervisor and workers"
            .to_string(),
        "  stop                                 Stop the active supervisor".to_string(),
        "  restart                              Restart the supervisor and workers".to_string(),
        "  status                               Show runtime ownership/health status".to_string(),
        "  logs                                 Print runtime and worker logs".to_string(),
        "  attach                               Attach to the active runtime session".to_string(),
        "  doctor                               Run local environment and config checks"
            .to_string(),
        "  update check|apply                   Check for updates (apply is intentionally blocked)"
            .to_string(),
        "  send <profile> <message>             Queue a message for a channel profile".to_string(),
        "  channels reset                       Reset channel sync state".to_string(),
        "  channels slack sync                  Pull Slack messages into the queue".to_string(),
        "  auth sync                            Sync provider auth from configured sources"
            .to_string(),
        "  orchestrator ...                     Manage orchestrators and routing defaults"
            .to_string(),
        "  orchestrator-agent ...               Manage agents under an orchestrator".to_string(),
        "  agent ...                            Alias for `orchestrator-agent ...`".to_string(),
        "  workflow ...                         Manage workflows and workflow runs".to_string(),
        "  channel-profile ...                  Manage channel-to-orchestrator bindings"
            .to_string(),
        "  provider ...                         Set/show default provider preference".to_string(),
        "  model ...                            Set/show default model preference".to_string(),
    ]
}
