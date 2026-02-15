use crate::config::{
    default_global_config_path, load_orchestrator_config,
    remove_orchestrator_config as config_remove_orchestrator_config,
    save_orchestrator_config as config_save_orchestrator_config,
    save_orchestrator_registry as config_save_orchestrator_registry,
    save_settings as config_save_settings, ConfigError, OrchestratorConfig, Settings,
    ValidationOptions,
};
use crate::runtime::{bootstrap_state_root, default_state_root_path, StatePaths};
use crate::workflow::{initial_orchestrator_config, WorkflowTemplate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimePreferences {
    pub provider: Option<String>,
    pub model: Option<String>,
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

pub fn map_config_err(err: ConfigError) -> String {
    err.to_string()
}

fn state_root() -> Result<PathBuf, String> {
    default_state_root_path().map_err(|e| e.to_string())
}

pub fn ensure_runtime_root() -> Result<StatePaths, String> {
    let root = state_root()?;
    let paths = StatePaths::new(root);
    bootstrap_state_root(&paths).map_err(|e| e.to_string())?;
    Ok(paths)
}

fn preferences_path(paths: &StatePaths) -> PathBuf {
    paths.root.join("runtime/preferences.yaml")
}

pub fn load_preferences(paths: &StatePaths) -> Result<RuntimePreferences, String> {
    let path = preferences_path(paths);
    if !path.exists() {
        return Ok(RuntimePreferences::default());
    }
    let raw =
        fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    serde_yaml::from_str(&raw).map_err(|e| format!("failed to parse {}: {e}", path.display()))
}

pub fn save_preferences(paths: &StatePaths, prefs: &RuntimePreferences) -> Result<(), String> {
    let path = preferences_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let body =
        serde_yaml::to_string(prefs).map_err(|e| format!("failed to encode preferences: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

pub fn load_settings() -> Result<Settings, String> {
    let path = default_global_config_path().map_err(map_config_err)?;
    let settings = Settings::from_path(&path).map_err(map_config_err)?;
    settings
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .map_err(map_config_err)?;
    Ok(settings)
}

pub fn save_settings(settings: &Settings) -> Result<PathBuf, String> {
    config_save_settings(settings).map_err(map_config_err)
}

pub fn save_orchestrator_config(
    settings: &Settings,
    orchestrator_id: &str,
    orchestrator: &OrchestratorConfig,
) -> Result<PathBuf, String> {
    config_save_orchestrator_config(settings, orchestrator_id, orchestrator).map_err(map_config_err)
}

pub fn save_orchestrator_registry(
    settings: &Settings,
    registry: &BTreeMap<String, OrchestratorConfig>,
) -> Result<PathBuf, String> {
    config_save_orchestrator_registry(settings, registry).map_err(map_config_err)
}

pub fn remove_orchestrator_config(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<(), String> {
    config_remove_orchestrator_config(settings, orchestrator_id).map_err(map_config_err)
}

pub fn load_orchestrator_or_err(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<OrchestratorConfig, String> {
    load_orchestrator_config(settings, orchestrator_id).map_err(map_config_err)
}

pub fn default_orchestrator_config(id: &str) -> OrchestratorConfig {
    initial_orchestrator_config(id, "anthropic", "sonnet", WorkflowTemplate::Minimal)
}
