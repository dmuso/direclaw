use direclaw::config::error::ConfigError;

#[test]
fn config_error_module_exposes_config_error_type() {
    let _: fn(String) -> ConfigError = ConfigError::Settings;
}
