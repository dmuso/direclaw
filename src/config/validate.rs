use super::{ConfigError, OrchestratorConfig, Settings, ValidationOptions};

pub fn validate_settings(
    settings: &Settings,
    options: ValidationOptions,
) -> Result<(), ConfigError> {
    settings.validate(options)
}

pub fn validate_orchestrator_config(
    orchestrator: &OrchestratorConfig,
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<(), ConfigError> {
    orchestrator.validate(settings, orchestrator_id)
}
