use std::fs;
use std::path::Path;

#[test]
fn app_sources_do_not_depend_on_orchestrator_compat_module() {
    let app_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app");
    let mut pending = vec![app_dir];

    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("read app dir") {
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
                !source.contains("crate::orchestrator::"),
                "{} imports orchestration via compatibility module; use crate::orchestration::* modules directly",
                path.display()
            );
        }
    }
}
