use crate::app::command_support::{
    load_settings, map_config_err, save_orchestrator_registry, save_preferences, save_settings,
    RuntimePreferences,
};
use crate::config::{default_global_config_path, OrchestratorConfig, SettingsOrchestrator};
use crate::runtime::StatePaths;
use crate::setup::state::{infer_workflow_template, SetupState};
use crate::workflow::{initial_orchestrator_config, WorkflowTemplate as SetupWorkflowTemplate};
use std::collections::BTreeMap;
use std::fs;

pub(crate) fn load_setup_bootstrap(paths: &StatePaths) -> Result<SetupState, String> {
    let default_workspace = paths.root.join("workspaces");
    let mut bootstrap = SetupState {
        workspaces_path: default_workspace,
        orchestrator_id: "main".to_string(),
        provider: "anthropic".to_string(),
        model: "sonnet".to_string(),
        workflow_template: SetupWorkflowTemplate::Minimal,
        orchestrators: BTreeMap::from_iter([(
            "main".to_string(),
            SettingsOrchestrator {
                private_workspace: None,
                shared_access: Vec::new(),
            },
        )]),
        orchestrator_configs: BTreeMap::from_iter([(
            "main".to_string(),
            initial_orchestrator_config(
                "main",
                "anthropic",
                "sonnet",
                SetupWorkflowTemplate::Minimal,
            ),
        )]),
    };

    let config_path = default_global_config_path().map_err(map_config_err)?;
    if !config_path.exists() {
        return Ok(bootstrap);
    }

    let settings = load_settings()?;
    bootstrap.workspaces_path = settings.workspaces_path.clone();
    bootstrap.orchestrators = settings.orchestrators.clone();
    let mut configs = BTreeMap::new();
    for orchestrator_id in bootstrap.orchestrators.keys() {
        let private_workspace = settings
            .resolve_private_workspace(orchestrator_id)
            .map_err(map_config_err)?;
        let orchestrator_path = private_workspace.join("orchestrator.yaml");
        if orchestrator_path.exists() {
            let raw = fs::read_to_string(&orchestrator_path)
                .map_err(|e| format!("failed to read {}: {e}", orchestrator_path.display()))?;
            let config = serde_yaml::from_str::<OrchestratorConfig>(&raw)
                .map_err(|e| format!("failed to parse {}: {e}", orchestrator_path.display()))?;
            configs.insert(orchestrator_id.clone(), config);
        } else {
            configs.insert(
                orchestrator_id.clone(),
                initial_orchestrator_config(
                    orchestrator_id,
                    &bootstrap.provider,
                    &bootstrap.model,
                    SetupWorkflowTemplate::Minimal,
                ),
            );
        }
    }
    bootstrap.orchestrator_configs = configs;
    if let Some(first_orchestrator) = settings.orchestrators.keys().next() {
        bootstrap.orchestrator_id = first_orchestrator.clone();
        if let Some(orchestrator) = bootstrap.orchestrator_configs.get(first_orchestrator) {
            if let Some(selector) = orchestrator.agents.get(&orchestrator.selector_agent) {
                bootstrap.provider = selector.provider.to_string();
                bootstrap.model = selector.model.clone();
            } else if let Some((_, agent)) = orchestrator.agents.iter().next() {
                bootstrap.provider = agent.provider.to_string();
                bootstrap.model = agent.model.clone();
            }
            bootstrap.workflow_template = infer_workflow_template(orchestrator);
        }
    }

    Ok(bootstrap)
}

pub(crate) fn persist_setup_state(
    paths: &StatePaths,
    state: &mut SetupState,
    config_exists: bool,
) -> Result<String, String> {
    fs::create_dir_all(&state.workspaces_path).map_err(|e| {
        format!(
            "failed to create workspace {}: {e}",
            state.workspaces_path.display()
        )
    })?;

    let existing_settings = if config_exists {
        Some(load_settings()?)
    } else {
        None
    };
    let settings = state.normalize_for_save(existing_settings)?;
    let path = save_settings(&settings)?;
    save_orchestrator_registry(&settings, &state.orchestrator_configs)?;
    let orchestrator_path = settings
        .resolve_private_workspace(&state.orchestrator_id)
        .map_err(map_config_err)?
        .join("orchestrator.yaml");

    let prefs = RuntimePreferences {
        provider: Some(state.provider.clone()),
        model: Some(state.model.clone()),
    };
    save_preferences(paths, &prefs)?;

    Ok(format!(
        "setup complete\nconfig={}\nstate_root={}\nworkspace={}\norchestrator={}\nnew_workflow_template={}\nprovider={}\nmodel={}\norchestrator_config={}",
        path.display(),
        paths.root.display(),
        state.workspaces_path.display(),
        state.orchestrator_id,
        state.workflow_template.as_str(),
        state.provider,
        state.model,
        orchestrator_path.display()
    ))
}
