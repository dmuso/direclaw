use crate::commands::{
    default_orchestrator_config, load_orchestrator_or_err, load_settings, map_config_err,
    remove_orchestrator_config, save_orchestrator_config, save_settings,
};
use crate::config::SettingsOrchestrator;
use std::fs;
use std::path::PathBuf;

pub fn cmd_orchestrator(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("usage: orchestrator <list|add|show|remove|set-private-workspace|grant-shared-access|revoke-shared-access|set-selector-agent|set-default-workflow|set-selection-max-retries> ...".to_string());
    }

    match args[0].as_str() {
        "list" => {
            let settings = load_settings()?;
            Ok(settings
                .orchestrators
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "add" => {
            if args.len() != 2 {
                return Err("usage: orchestrator add <orchestrator_id>".to_string());
            }
            let mut settings = load_settings()?;
            let id = args[1].clone();
            if settings.orchestrators.contains_key(&id) {
                return Err(format!("orchestrator `{id}` already exists"));
            }
            settings.orchestrators.insert(
                id.clone(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            );
            save_settings(&settings)?;

            let private_workspace = settings
                .resolve_private_workspace(&id)
                .map_err(map_config_err)?;
            fs::create_dir_all(&private_workspace)
                .map_err(|e| format!("failed to create {}: {e}", private_workspace.display()))?;
            let path = save_orchestrator_config(&settings, &id, &default_orchestrator_config(&id))?;
            Ok(format!(
                "orchestrator added\nid={}\nprivate_workspace={}\nconfig={}",
                id,
                private_workspace.display(),
                path.display()
            ))
        }
        "show" => {
            if args.len() != 2 {
                return Err("usage: orchestrator show <orchestrator_id>".to_string());
            }
            let settings = load_settings()?;
            let id = &args[1];
            let entry = settings
                .orchestrators
                .get(id)
                .ok_or_else(|| format!("unknown orchestrator `{id}`"))?;
            let private_workspace = settings
                .resolve_private_workspace(id)
                .map_err(map_config_err)?;
            Ok(format!(
                "id={}\nprivate_workspace={}\nshared_access={}",
                id,
                private_workspace.display(),
                entry.shared_access.join(",")
            ))
        }
        "remove" => {
            if args.len() != 2 {
                return Err("usage: orchestrator remove <orchestrator_id>".to_string());
            }
            let mut settings = load_settings()?;
            let id = args[1].clone();
            if settings.orchestrators.remove(&id).is_none() {
                return Err(format!("unknown orchestrator `{id}`"));
            }
            save_settings(&settings)?;
            remove_orchestrator_config(&settings, &id)?;
            Ok(format!("orchestrator removed\nid={id}"))
        }
        "set-private-workspace" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator set-private-workspace <orchestrator_id> <abs_path>"
                        .to_string(),
                );
            }
            let mut settings = load_settings()?;
            let id = &args[1];
            let orchestrator = load_orchestrator_or_err(&settings, id)?;
            let path = PathBuf::from(&args[2]);
            if !path.is_absolute() {
                return Err("private workspace path must be absolute".to_string());
            }
            let entry = settings
                .orchestrators
                .get_mut(id)
                .ok_or_else(|| format!("unknown orchestrator `{id}`"))?;
            entry.private_workspace = Some(path.clone());
            save_settings(&settings)?;
            save_orchestrator_config(&settings, id, &orchestrator)?;
            Ok(format!(
                "orchestrator updated\nid={}\nprivate_workspace={}",
                id,
                path.display()
            ))
        }
        "grant-shared-access" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator grant-shared-access <orchestrator_id> <shared_key>"
                        .to_string(),
                );
            }
            let mut settings = load_settings()?;
            let id = &args[1];
            let shared_key = args[2].clone();
            if !settings.shared_workspaces.contains_key(&shared_key) {
                return Err(format!("invalid shared key `{shared_key}`"));
            }
            let entry = settings
                .orchestrators
                .get_mut(id)
                .ok_or_else(|| format!("unknown orchestrator `{id}`"))?;
            if !entry.shared_access.contains(&shared_key) {
                entry.shared_access.push(shared_key.clone());
            }
            save_settings(&settings)?;
            Ok(format!(
                "shared access granted\nid={}\nshared_key={}",
                id, shared_key
            ))
        }
        "revoke-shared-access" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator revoke-shared-access <orchestrator_id> <shared_key>"
                        .to_string(),
                );
            }
            let mut settings = load_settings()?;
            let id = &args[1];
            let shared_key = args[2].clone();
            let entry = settings
                .orchestrators
                .get_mut(id)
                .ok_or_else(|| format!("unknown orchestrator `{id}`"))?;
            entry.shared_access.retain(|v| v != &shared_key);
            save_settings(&settings)?;
            Ok(format!(
                "shared access revoked\nid={}\nshared_key={}",
                id, shared_key
            ))
        }
        "set-selector-agent" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator set-selector-agent <orchestrator_id> <agent_id>"
                        .to_string(),
                );
            }
            let settings = load_settings()?;
            let id = &args[1];
            let agent_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, id)?;
            if !orchestrator.agents.contains_key(&agent_id) {
                return Err(format!("unknown agent `{agent_id}`"));
            }
            orchestrator.selector_agent = agent_id.clone();
            save_orchestrator_config(&settings, id, &orchestrator)?;
            Ok(format!(
                "selector updated\nid={}\nselector_agent={}",
                id, agent_id
            ))
        }
        "set-default-workflow" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator set-default-workflow <orchestrator_id> <workflow_id>"
                        .to_string(),
                );
            }
            let settings = load_settings()?;
            let id = &args[1];
            let workflow_id = args[2].clone();
            let mut orchestrator = load_orchestrator_or_err(&settings, id)?;
            if !orchestrator.workflows.iter().any(|w| w.id == workflow_id) {
                return Err(format!("invalid workflow id `{workflow_id}`"));
            }
            orchestrator.default_workflow = workflow_id.clone();
            save_orchestrator_config(&settings, id, &orchestrator)?;
            Ok(format!(
                "default workflow updated\nid={}\ndefault_workflow={}",
                id, workflow_id
            ))
        }
        "set-selection-max-retries" => {
            if args.len() != 3 {
                return Err(
                    "usage: orchestrator set-selection-max-retries <orchestrator_id> <count>"
                        .to_string(),
                );
            }
            let settings = load_settings()?;
            let id = &args[1];
            let count: u32 = args[2]
                .parse()
                .map_err(|_| "count must be a positive integer".to_string())?;
            if count == 0 {
                return Err("count must be >= 1".to_string());
            }
            let mut orchestrator = load_orchestrator_or_err(&settings, id)?;
            orchestrator.selection_max_retries = count;
            save_orchestrator_config(&settings, id, &orchestrator)?;
            Ok(format!(
                "selection retries updated\nid={}\ncount={}",
                id, count
            ))
        }
        other => Err(format!("unknown orchestrator subcommand `{other}`")),
    }
}
