use std::path::Path;

#[test]
fn queue_module_root_uses_directory_mod_layout() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        root.join("src/queue/mod.rs").exists(),
        "expected src/queue/mod.rs to exist as the queue module root"
    );
}
