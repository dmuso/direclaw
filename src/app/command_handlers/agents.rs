use crate::app::command_support::{
    load_orchestrator_or_err, load_settings, save_orchestrator_config,
};
use crate::config::{AgentConfig, ConfigProviderKind};

pub fn cmd_orchestrator_agent(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("usage: orchestrator-agent <list|add|show|remove|reset> ...".to_string());
    }

    match args[0].as_str() {
        "list" => {
            if args.len() != 2 {
                return Err("usage: orchestrator-agent list <orchestrator_id>".to_string());
            }
            let settings = load_settings()?;
            let orchestrator = load_orchestrator_or_err(&settings, &args[1])?;
            Ok(orchestrator
                .agents
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "add" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator-agent add <orchestrator_id> <agent_id>".to_string()
                );
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let agent_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            if orchestrator.agents.contains_key(&agent_id) {
                return Err(format!("agent `{agent_id}` already exists"));
            }
            orchestrator.agents.insert(
                agent_id.clone(),
                AgentConfig {
                    provider: ConfigProviderKind::Anthropic,
                    model: "sonnet".to_string(),
                    can_orchestrate_workflows: false,
                },
            );
            save_orchestrator_config(&settings, orchestrator_id, &orchestrator)?;
            Ok(format!(
                "agent added\norchestrator={}\nagent={}",
                orchestrator_id, agent_id
            ))
        }
        "show" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator-agent show <orchestrator_id> <agent_id>".to_string(),
                );
            }
            let settings = load_settings()?;
            let orchestrator = load_orchestrator_or_err(&settings, &args[1])?;
            let agent = orchestrator
                .agents
                .get(&args[2])
                .ok_or_else(|| format!("unknown agent `{}`", args[2]))?;
            Ok(format!(
                "id={}\nprovider={}\nmodel={}\ncan_orchestrate_workflows={}",
                args[2], agent.provider, agent.model, agent.can_orchestrate_workflows
            ))
        }
        "remove" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator-agent remove <orchestrator_id> <agent_id>".to_string(),
                );
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let agent_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            if orchestrator.agents.remove(&agent_id).is_none() {
                return Err(format!("unknown agent `{agent_id}`"));
            }
            save_orchestrator_config(&settings, orchestrator_id, &orchestrator)?;
            Ok(format!(
                "agent removed\norchestrator={}\nagent={}",
                orchestrator_id, agent_id
            ))
        }
        "reset" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator-agent reset <orchestrator_id> <agent_id>".to_string(),
                );
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let agent_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, orchestrator_id)?;
            let agent = orchestrator
                .agents
                .get_mut(&agent_id)
                .ok_or_else(|| format!("unknown agent `{agent_id}`"))?;
            agent.provider = ConfigProviderKind::Anthropic;
            agent.model = "sonnet".to_string();
            agent.can_orchestrate_workflows = false;
            save_orchestrator_config(&settings, orchestrator_id, &orchestrator)?;
            Ok(format!(
                "agent reset\norchestrator={}\nagent={}",
                orchestrator_id, agent_id
            ))
        }
        other => Err(format!("unknown orchestrator-agent subcommand `{other}`")),
    }
}
