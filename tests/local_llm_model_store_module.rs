use direclaw::local_llm::{
    ensure_model_available_with_downloader, resolve_model_location, LocalLlmModelConfig,
};
use std::cell::Cell;
use std::fs;
use tempfile::tempdir;

#[test]
fn model_location_scopes_under_state_root_models() {
    let root = tempdir().expect("tempdir");
    let model = LocalLlmModelConfig::default();
    let location = resolve_model_location(root.path(), &model);
    assert_eq!(location.models_dir, root.path().join("models"));
    assert_eq!(
        location.model_path,
        root.path().join("models/Qwen3.5-0.8B-UD-IQ2_M.gguf")
    );
}

#[test]
fn ensure_model_skips_download_when_file_and_manifest_match() {
    let root = tempdir().expect("tempdir");
    let model = LocalLlmModelConfig::default();
    let location = resolve_model_location(root.path(), &model);
    fs::create_dir_all(&location.models_dir).expect("mkdir");
    fs::write(&location.model_path, b"gguf").expect("write model");
    fs::write(
        &location.manifest_path,
        r#"{
  "repo": "unsloth/Qwen3.5-0.8B-GGUF",
  "file": "Qwen3.5-0.8B-UD-IQ2_M.gguf",
  "revision": "main",
  "downloadedAt": 1,
  "sizeBytes": 4,
  "sha256": "abc"
}"#,
    )
    .expect("write manifest");

    let called = Cell::new(false);
    let path = ensure_model_available_with_downloader(root.path(), &model, |_url, _target| {
        called.set(true);
        Ok(())
    })
    .expect("ensure model");

    assert_eq!(path, location.model_path);
    assert!(
        !called.get(),
        "downloader should not be called when model and manifest match"
    );
}

#[test]
fn ensure_model_downloads_when_missing() {
    let root = tempdir().expect("tempdir");
    let model = LocalLlmModelConfig::default();
    let location = resolve_model_location(root.path(), &model);

    let path = ensure_model_available_with_downloader(root.path(), &model, |_url, target| {
        fs::write(target, b"gguf-data")
            .map_err(|e| format!("failed to write fake download {}: {e}", target.display()))
    })
    .expect("ensure model");

    assert_eq!(path, location.model_path);
    assert!(location.model_path.is_file());
    assert!(location.manifest_path.is_file());
}

#[test]
fn ensure_model_fails_when_download_does_not_create_file() {
    let root = tempdir().expect("tempdir");
    let model = LocalLlmModelConfig::default();

    let err = ensure_model_available_with_downloader(root.path(), &model, |_url, _target| Ok(()))
        .expect_err("ensure model should fail");

    assert!(err.contains("temp file is missing"), "err={err}");
}

#[test]
fn ensure_model_redownloads_when_manifest_revision_mismatches() {
    let root = tempdir().expect("tempdir");
    let model = LocalLlmModelConfig {
        revision: Some("rev-a".to_string()),
        ..LocalLlmModelConfig::default()
    };
    let location = resolve_model_location(root.path(), &model);
    fs::create_dir_all(&location.models_dir).expect("mkdir");
    fs::write(&location.model_path, b"old-model").expect("write old model");
    fs::write(
        &location.manifest_path,
        r#"{
  "repo": "unsloth/Qwen3.5-0.8B-GGUF",
  "file": "Qwen3.5-0.8B-UD-IQ2_M.gguf",
  "revision": "rev-old",
  "downloadedAt": 1,
  "sizeBytes": 10,
  "sha256": "abc"
}"#,
    )
    .expect("write old manifest");

    let path = ensure_model_available_with_downloader(root.path(), &model, |_url, target| {
        fs::write(target, b"new-model")
            .map_err(|e| format!("failed to write fake download {}: {e}", target.display()))
    })
    .expect("ensure model");
    assert_eq!(path, location.model_path);
    let body = fs::read(&location.model_path).expect("read model");
    assert_eq!(body, b"new-model");
}

#[test]
fn ensure_model_redownloads_when_manifest_is_missing() {
    let root = tempdir().expect("tempdir");
    let model = LocalLlmModelConfig::default();
    let location = resolve_model_location(root.path(), &model);
    fs::create_dir_all(&location.models_dir).expect("mkdir");
    fs::write(&location.model_path, b"old-model").expect("write old model");

    let path = ensure_model_available_with_downloader(root.path(), &model, |_url, target| {
        fs::write(target, b"replacement")
            .map_err(|e| format!("failed to write fake download {}: {e}", target.display()))
    })
    .expect("ensure model");
    assert_eq!(path, location.model_path);
    let body = fs::read(&location.model_path).expect("read model");
    assert_eq!(body, b"replacement");
    assert!(
        location.manifest_path.is_file(),
        "manifest should be written"
    );
}
