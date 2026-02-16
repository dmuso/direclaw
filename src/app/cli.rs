use crate::app::command_catalog::{canonical_cli_tokens, V1_FUNCTIONS};

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

pub fn normalize_cli_args(args: Vec<String>) -> Vec<String> {
    if args.is_empty() {
        return args;
    }

    let Some((scope_raw, action_raw)) = args[0].split_once('.') else {
        return args;
    };
    if scope_raw.is_empty() || action_raw.is_empty() {
        return args;
    }

    let scope = scope_raw.replace('_', "-");
    let mut normalized = Vec::with_capacity(args.len() + 1);
    if scope == "daemon" {
        normalized.push(action_raw.replace('_', "-"));
    } else if scope == "channels" && action_raw == "slack_sync" {
        normalized.push(scope);
        normalized.push("slack".to_string());
        normalized.push("sync".to_string());
    } else {
        normalized.push(scope);
        normalized.push(action_raw.replace('_', "-"));
    }
    normalized.extend_from_slice(&args[1..]);
    normalized
}

pub fn cli_help_lines() -> Vec<String> {
    let mut defs: Vec<_> = V1_FUNCTIONS.iter().collect();
    defs.sort_by(|a, b| a.function_id.cmp(b.function_id));

    let mut lines = vec!["Commands:".to_string()];
    for def in defs {
        if let Some(tokens) = canonical_cli_tokens(def.function_id) {
            lines.push(format!("  {0:36} {1}", tokens.join(" "), def.description));
        }
    }
    lines
}

pub(crate) fn help_text() -> String {
    cli_help_lines().join("\n")
}

#[cfg(test)]
mod tests {
    use super::normalize_cli_args;

    #[test]
    fn normalize_cli_args_maps_selector_daemon_aliases_to_cli_verbs() {
        let args = vec!["daemon.start".to_string()];
        assert_eq!(normalize_cli_args(args), vec!["start".to_string()]);
    }

    #[test]
    fn normalize_cli_args_maps_selector_channel_profile_aliases() {
        let args = vec!["channel_profile.list".to_string()];
        assert_eq!(
            normalize_cli_args(args),
            vec!["channel-profile".to_string(), "list".to_string()]
        );
    }

    #[test]
    fn normalize_cli_args_maps_channels_slack_sync_nested_alias() {
        let args = vec!["channels.slack_sync".to_string()];
        assert_eq!(
            normalize_cli_args(args),
            vec![
                "channels".to_string(),
                "slack".to_string(),
                "sync".to_string()
            ]
        );
    }
}
