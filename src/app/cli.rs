use crate::app::command_catalog::V1_FUNCTIONS;

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

pub fn selector_help_lines() -> Vec<String> {
    let mut defs: Vec<_> = V1_FUNCTIONS.iter().collect();
    defs.sort_by(|a, b| a.function_id.cmp(b.function_id));
    defs.into_iter()
        .map(|def| format!("  {0:36} {1}", def.function_id, def.description))
        .collect()
}

pub(crate) fn help_text() -> String {
    let mut lines = cli_help_lines();
    lines.push(String::new());
    lines.push("Selector-callable operations:".to_string());
    lines.extend(selector_help_lines());
    lines.join("\n")
}
