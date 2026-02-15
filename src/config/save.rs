use super::{
    default_global_config_path, ConfigError, OrchestratorConfig, Settings, ValidationOptions,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn create_parent_dir(path: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDir {
            path: parent.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

pub fn save_settings(settings: &Settings) -> Result<PathBuf, ConfigError> {
    settings.validate(ValidationOptions {
        require_shared_paths_exist: false,
    })?;

    let path = default_global_config_path()?;
    create_parent_dir(&path)?;
    let body = serde_yaml::to_string(settings).map_err(|source| ConfigError::Encode {
        path: path.display().to_string(),
        source,
    })?;
    fs::write(&path, body).map_err(|source| ConfigError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}

pub fn save_orchestrator_config(
    settings: &Settings,
    orchestrator_id: &str,
    orchestrator: &OrchestratorConfig,
) -> Result<PathBuf, ConfigError> {
    orchestrator.validate(settings, orchestrator_id)?;
    let private_workspace = settings.resolve_private_workspace(orchestrator_id)?;
    fs::create_dir_all(&private_workspace).map_err(|source| ConfigError::CreateDir {
        path: private_workspace.display().to_string(),
        source,
    })?;
    let path = private_workspace.join("orchestrator.yaml");
    let body = serde_yaml::to_string(orchestrator).map_err(|source| ConfigError::Encode {
        path: path.display().to_string(),
        source,
    })?;
    fs::write(&path, body).map_err(|source| ConfigError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(path)
}

pub fn save_orchestrator_registry(
    settings: &Settings,
    registry: &BTreeMap<String, OrchestratorConfig>,
) -> Result<PathBuf, ConfigError> {
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
    fs::remove_file(&path).map_err(|source| ConfigError::Write {
        path: path.display().to_string(),
        source,
    })
}
