use std::fs;
use std::path::Path;

#[test]
fn lib_root_does_not_export_legacy_cli_module() {
    let lib_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let source = fs::read_to_string(&lib_rs).expect("read src/lib.rs");

    assert!(
        !source.contains("pub mod cli;"),
        "src/lib.rs still exports legacy root cli module; binary should route through app modules"
    );
}

#[test]
fn lib_root_does_not_export_legacy_orchestrator_module() {
    let lib_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let source = fs::read_to_string(&lib_rs).expect("read src/lib.rs");

    assert!(
        !source.contains("pub mod orchestrator;"),
        "src/lib.rs still exports legacy root orchestrator module; orchestration should be consumed through src/orchestration/*"
    );

    let legacy_orchestrator = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/orchestrator.rs");
    assert!(
        !legacy_orchestrator.exists(),
        "legacy compatibility module still exists at src/orchestrator.rs"
    );
}
