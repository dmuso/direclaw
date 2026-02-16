use crate::orchestration::selector::FunctionArgType;

pub type FunctionArgTypeDef = FunctionArgType;

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
    pub const DAEMON_CHAT: &str = "daemon.chat";
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
    pub const DAEMON_DOCTOR: &str = "daemon.doctor";
    pub const AUTH_SYNC: &str = "auth.sync";
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
        function_id: function_ids::DAEMON_CHAT,
        description: "Start local chat REPL for a local channel profile",
        args: &[FunctionArgDef {
            name: "channelProfileId",
            arg_type: FunctionArgTypeDef::String,
            required: true,
            description: "Local channel profile id",
        }],
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
    FunctionDef {
        function_id: function_ids::DAEMON_DOCTOR,
        description: "Run local environment and config checks",
        args: &[],
        read_only: true,
    },
    FunctionDef {
        function_id: function_ids::AUTH_SYNC,
        description: "Sync provider auth from configured sources",
        args: &[],
        read_only: false,
    },
];

pub fn function_def(function_id: &str) -> Option<&'static FunctionDef> {
    V1_FUNCTIONS
        .iter()
        .find(|def| def.function_id == function_id)
}

pub fn canonical_cli_tokens(function_id: &str) -> Option<Vec<String>> {
    let (scope_raw, action_raw) = function_id.split_once('.')?;
    if scope_raw.is_empty() || action_raw.is_empty() {
        return None;
    }

    if scope_raw == "daemon" {
        return Some(vec![action_raw.replace('_', "-")]);
    }

    let scope = scope_raw.replace('_', "-");
    if scope_raw == "channels" && action_raw == "slack_sync" {
        return Some(vec![scope, "slack".to_string(), "sync".to_string()]);
    }

    Some(vec![scope, action_raw.replace('_', "-")])
}
