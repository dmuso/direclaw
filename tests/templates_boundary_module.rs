use std::fs;
use std::path::Path;

#[test]
fn lib_rs_does_not_expose_legacy_workflow_compat_module() {
    let lib_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let source = fs::read_to_string(&lib_rs).expect("read lib.rs");
    assert!(
        !source.contains("pub mod workflow;"),
        "lib.rs still exposes legacy `workflow` compatibility module"
    );
}

#[test]
fn source_tree_does_not_use_legacy_workflow_module_path() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut pending = vec![src_dir];

    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("read source dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }

            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }

            let source = fs::read_to_string(&path).expect("read source");
            assert!(
                !source.contains("crate::workflow"),
                "{} imports via legacy crate::workflow path",
                path.display()
            );
        }
    }
}
