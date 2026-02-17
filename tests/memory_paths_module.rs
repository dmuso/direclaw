use direclaw::memory::{bootstrap_memory_paths, MemoryPathError, MemoryPaths};
use std::fs;
use tempfile::tempdir;

#[test]
fn memory_paths_module_resolves_runtime_subpaths_deterministically() {
    let root = tempdir().expect("tempdir");
    let runtime_root = root.path().join("orchestrator-a");
    let paths = MemoryPaths::from_runtime_root(&runtime_root);

    assert_eq!(paths.root, runtime_root.join("memory"));
    assert_eq!(paths.database, runtime_root.join("memory/memory.db"));
    assert_eq!(paths.ingest, runtime_root.join("memory/ingest"));
    assert_eq!(
        paths.ingest_processed,
        runtime_root.join("memory/ingest/processed")
    );
    assert_eq!(
        paths.ingest_rejected,
        runtime_root.join("memory/ingest/rejected")
    );
    assert_eq!(paths.bulletins, runtime_root.join("memory/bulletins"));
    assert_eq!(paths.logs_dir, runtime_root.join("memory/logs"));
    assert_eq!(paths.log_file, runtime_root.join("memory/logs/memory.log"));
}

#[test]
fn memory_paths_module_can_canonicalize_runtime_root_before_joining() {
    let root = tempdir().expect("tempdir");
    let runtime_root = root.path().join("orch");
    fs::create_dir_all(&runtime_root).expect("create runtime root");

    let with_dot = runtime_root.join(".");
    let paths = MemoryPaths::from_runtime_root_canonical(&with_dot)
        .expect("resolve canonical memory paths");

    assert_eq!(paths.root, runtime_root.join("memory"));
}

#[test]
fn memory_paths_module_bootstraps_required_directories() {
    let root = tempdir().expect("tempdir");
    let runtime_root = root.path().join("orch");
    fs::create_dir_all(&runtime_root).expect("runtime root");
    let paths = MemoryPaths::from_runtime_root(&runtime_root);

    bootstrap_memory_paths(&paths).expect("bootstrap memory paths");
    for required in paths.required_directories() {
        assert!(
            required.is_dir(),
            "missing memory runtime directory {}",
            required.display()
        );
    }
}

#[test]
fn memory_paths_module_returns_typed_create_dir_error_with_path_context() {
    let root = tempdir().expect("tempdir");
    let runtime_root = root.path().join("orch");
    fs::create_dir_all(&runtime_root).expect("runtime root");
    fs::write(runtime_root.join("memory"), "not a directory").expect("write file");

    let paths = MemoryPaths::from_runtime_root(&runtime_root);
    let err = bootstrap_memory_paths(&paths).expect_err("bootstrap should fail");

    match err {
        MemoryPathError::CreateDir { path, .. } => {
            assert!(path.contains("memory"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
