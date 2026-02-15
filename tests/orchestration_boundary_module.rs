use std::fs;
use std::path::Path;

#[test]
fn orchestration_sources_do_not_import_orchestrator_error_via_compat_module() {
    let orchestration_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/orchestration");

    for entry in fs::read_dir(&orchestration_dir).expect("read orchestration dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("error.rs") {
            continue;
        }

        let source = fs::read_to_string(&path).expect("read source");
        assert!(
            !source.contains("crate::orchestrator::OrchestratorError"),
            "{} imports OrchestratorError via compatibility module; use crate::orchestration::error::OrchestratorError",
            path.display()
        );
    }
}

#[test]
fn orchestration_sources_do_not_depend_on_app_module() {
    let orchestration_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/orchestration");

    for entry in fs::read_dir(&orchestration_dir).expect("read orchestration dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }

        let source = fs::read_to_string(&path).expect("read source");
        assert!(
            !source.contains("crate::app::"),
            "{} imports app-layer modules; orchestration must not depend on app",
            path.display()
        );
    }
}
