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

#[test]
fn lib_root_does_not_export_legacy_commands_module() {
    let lib_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let source = fs::read_to_string(&lib_rs).expect("read src/lib.rs");

    assert!(
        !source.contains("pub mod commands;"),
        "src/lib.rs still exports legacy root commands module; command surfaces should be consumed through src/app/*"
    );

    let legacy_commands = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands.rs");
    assert!(
        !legacy_commands.exists(),
        "legacy compatibility module still exists at src/commands.rs"
    );
}

#[test]
fn lib_root_does_not_export_legacy_tui_module() {
    let lib_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let source = fs::read_to_string(&lib_rs).expect("read src/lib.rs");

    assert!(
        !source.contains("pub mod tui;"),
        "src/lib.rs still exports legacy tui compatibility module; setup should be consumed through src/setup/*"
    );

    let legacy_tui_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui");
    assert!(
        !legacy_tui_dir.join("mod.rs").exists(),
        "legacy compatibility module still exists at src/tui/mod.rs"
    );
    assert!(
        !legacy_tui_dir.join("setup.rs").exists(),
        "legacy compatibility module still exists at src/tui/setup.rs"
    );
}
