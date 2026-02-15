use crate::config::ConfigError;
use std::path::PathBuf;

pub const GLOBAL_STATE_DIR: &str = ".direclaw";
pub const GLOBAL_SETTINGS_FILE_NAME: &str = "config.yaml";
pub const GLOBAL_ORCHESTRATORS_FILE_NAME: &str = "config-orchestrators.yaml";

pub fn default_global_config_path() -> Result<PathBuf, ConfigError> {
    let home = std::env::var_os("HOME").ok_or(ConfigError::HomeDirectoryUnavailable)?;
    Ok(PathBuf::from(home)
        .join(GLOBAL_STATE_DIR)
        .join(GLOBAL_SETTINGS_FILE_NAME))
}

pub fn default_orchestrators_config_path() -> Result<PathBuf, ConfigError> {
    let home = std::env::var_os("HOME").ok_or(ConfigError::HomeDirectoryUnavailable)?;
    Ok(PathBuf::from(home)
        .join(GLOBAL_STATE_DIR)
        .join(GLOBAL_ORCHESTRATORS_FILE_NAME))
}
