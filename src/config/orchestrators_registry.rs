use super::{save_orchestrator_config, ConfigError, OrchestratorConfig, Settings};
use std::collections::BTreeMap;

pub fn save_orchestrator_registry(
    settings: &Settings,
    registry: &BTreeMap<String, OrchestratorConfig>,
) -> Result<std::path::PathBuf, ConfigError> {
    let mut saved = None;
    for (orchestrator_id, orchestrator) in registry {
        let path = save_orchestrator_config(settings, orchestrator_id, orchestrator)?;
        saved = Some(path);
    }
    saved.ok_or_else(|| ConfigError::Settings("no orchestrator configs to save".to_string()))
}

pub fn remove_orchestrator_config(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<(), ConfigError> {
    let private_workspace = settings.resolve_private_workspace(orchestrator_id)?;
    let path = private_workspace.join("orchestrator.yaml");
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path).map_err(|source| ConfigError::Write {
        path: path.display().to_string(),
        source,
    })
}
