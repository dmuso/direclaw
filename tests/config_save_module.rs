use direclaw::config::{ConfigError, OrchestratorConfig, Settings};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[test]
fn config_save_module_exposes_persistence_entry_points() {
    let _save_settings: fn(&Settings) -> Result<PathBuf, ConfigError> =
        direclaw::config::save::save_settings;
    let _save_orchestrator: fn(
        &Settings,
        &str,
        &OrchestratorConfig,
    ) -> Result<PathBuf, ConfigError> = direclaw::config::save::save_orchestrator_config;
    let _save_registry: fn(
        &Settings,
        &BTreeMap<String, OrchestratorConfig>,
    ) -> Result<PathBuf, ConfigError> = direclaw::config::save::save_orchestrator_registry;
    let _remove_orchestrator: fn(&Settings, &str) -> Result<(), ConfigError> =
        direclaw::config::save::remove_orchestrator_config;
}
