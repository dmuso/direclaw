use super::{
    default_global_config_path, ConfigError, OrchestratorConfig, Settings, ValidationOptions,
};

pub fn load_global_settings() -> Result<Settings, ConfigError> {
    let path = default_global_config_path()?;
    let settings = Settings::from_path(&path)?;
    settings.validate(ValidationOptions::default())?;
    Ok(settings)
}

pub fn load_orchestrator_config(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<OrchestratorConfig, ConfigError> {
    if !settings.orchestrators.contains_key(orchestrator_id) {
        return Err(ConfigError::MissingOrchestrator {
            orchestrator_id: orchestrator_id.to_string(),
        });
    }
    let workspace = settings.resolve_private_workspace(orchestrator_id)?;
    let workspaces_path = workspace.join("orchestrator.yaml");
    let config = OrchestratorConfig::from_path(&workspaces_path)?;
    config.validate(settings, orchestrator_id)?;
    Ok(config)
}
