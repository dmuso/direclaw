use std::path::Path;

#[test]
fn config_module_root_uses_directory_mod_layout() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        root.join("src/config/mod.rs").exists(),
        "expected src/config/mod.rs to exist as the config module root"
    );
}
