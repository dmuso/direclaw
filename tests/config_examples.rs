use direclaw::config::{OrchestratorConfig, Settings, ValidationOptions};
use std::path::Path;

fn example_path(relative: &str) -> String {
    let primary = format!("docs/spec/{relative}");
    if Path::new(&primary).exists() {
        return primary;
    }
    format!("docs/build/spec/{relative}")
}

#[test]
fn settings_examples_parse_and_validate_cross_references() {
    let minimal = Settings::from_path(Path::new(&example_path(
        "examples/settings/minimal.settings.yaml",
    )))
    .expect("load minimal settings");
    minimal
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .expect("validate minimal settings");

    let full = Settings::from_path(Path::new(&example_path(
        "examples/settings/full.settings.yaml",
    )))
    .expect("load full settings");
    full.validate(ValidationOptions {
        require_shared_paths_exist: false,
    })
    .expect("validate full settings");
}

#[test]
fn orchestrator_examples_parse_and_validate_required_fields() {
    let settings = Settings::from_path(Path::new(&example_path(
        "examples/settings/full.settings.yaml",
    )))
    .expect("load full settings");

    let engineering = OrchestratorConfig::from_path(Path::new(&example_path(
        "examples/orchestrators/engineering.orchestrator.yaml",
    )))
    .expect("load engineering orchestrator");
    engineering
        .validate(&settings, "engineering_orchestrator")
        .expect("validate engineering orchestrator");

    let product = OrchestratorConfig::from_path(Path::new(&example_path(
        "examples/orchestrators/product.orchestrator.yaml",
    )))
    .expect("load product orchestrator");
    product
        .validate(&settings, "product_orchestrator")
        .expect("validate product orchestrator");

    let minimal_settings = Settings::from_path(Path::new(&example_path(
        "examples/settings/minimal.settings.yaml",
    )))
    .expect("load minimal settings");
    let minimal = OrchestratorConfig::from_path(Path::new(&example_path(
        "examples/orchestrators/minimal.orchestrator.yaml",
    )))
    .expect("load minimal orchestrator");
    minimal
        .validate(&minimal_settings, "default_orchestrator")
        .expect("validate minimal orchestrator");
}
