use std::path::Path;

#[test]
fn provider_module_root_uses_directory_mod_layout() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        root.join("src/provider/mod.rs").exists(),
        "expected src/provider/mod.rs to exist as the provider module root"
    );
}
