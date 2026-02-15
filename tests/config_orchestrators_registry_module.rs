use direclaw::config::{ConfigError, OrchestratorConfig, Settings};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[test]
fn config_orchestrators_registry_module_exposes_registry_entry_points() {
    let _save_registry: fn(
        &Settings,
        &BTreeMap<String, OrchestratorConfig>,
    ) -> Result<PathBuf, ConfigError> =
        direclaw::config::orchestrators_registry::save_orchestrator_registry;
    let _remove_orchestrator: fn(&Settings, &str) -> Result<(), ConfigError> =
        direclaw::config::orchestrators_registry::remove_orchestrator_config;
}
