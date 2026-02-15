use std::fs;
use std::path::Path;

#[test]
fn runtime_sources_do_not_depend_on_orchestrator_compat_module() {
    let runtime_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime");

    for entry in fs::read_dir(&runtime_dir).expect("read runtime dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
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

#[test]
fn runtime_sources_do_not_depend_on_slack_compat_module() {
    let runtime_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime");

    for entry in fs::read_dir(&runtime_dir).expect("read runtime dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }

        let source = fs::read_to_string(&path).expect("read source");
        assert!(
            !source.contains("use crate::slack"),
            "{} imports slack via compatibility module; use crate::channels::slack instead",
            path.display()
        );
    }
}

#[test]
fn runtime_mod_no_longer_hosts_worker_loop_sleep_helper() {
    let runtime_mod = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runtime/mod.rs");
    let source = fs::read_to_string(&runtime_mod).expect("read runtime mod");

    assert!(
        !source.contains("fn sleep_with_stop"),
        "src/runtime/mod.rs still owns worker-loop sleep logic; keep loop helpers with runtime/channel_worker"
    );
}
