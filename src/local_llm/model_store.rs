use crate::local_llm::LocalLlmModelConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MANIFEST_FILE: &str = "manifest.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelLocation {
    pub models_dir: PathBuf,
    pub model_path: PathBuf,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelManifest {
    repo: String,
    file: String,
    revision: String,
    downloaded_at: i64,
    size_bytes: u64,
    sha256: String,
}

pub fn resolve_model_location(state_root: &Path, model: &LocalLlmModelConfig) -> ModelLocation {
    let models_dir = state_root.join("models");
    let model_path = models_dir.join(model.file.trim());
    let manifest_path = models_dir.join(MANIFEST_FILE);
    ModelLocation {
        models_dir,
        model_path,
        manifest_path,
    }
}

pub fn ensure_model_available(
    state_root: &Path,
    model: &LocalLlmModelConfig,
) -> Result<PathBuf, String> {
    ensure_model_available_with_downloader(state_root, model, download_model_file)
}

pub fn ensure_model_available_with_downloader<F>(
    state_root: &Path,
    model: &LocalLlmModelConfig,
    downloader: F,
) -> Result<PathBuf, String>
where
    F: Fn(&str, &Path) -> Result<(), String>,
{
    let location = resolve_model_location(state_root, model);
    fs::create_dir_all(&location.models_dir).map_err(|e| {
        format!(
            "failed to create models directory {}: {e}",
            location.models_dir.display()
        )
    })?;

    let revision = model.revision.clone().unwrap_or_else(|| "main".to_string());
    if location.model_path.is_file() {
        if cached_model_matches(&location, model, &revision)? {
            validate_model_file(&location.model_path, model)?;
            return Ok(location.model_path);
        }
        fs::remove_file(&location.model_path).map_err(|e| {
            format!(
                "failed to remove stale model file {}: {e}",
                location.model_path.display()
            )
        })?;
    }

    let url = model_download_url(&model.repo, &revision, &model.file);

    let temp_path = location
        .model_path
        .with_extension(format!("downloading-{}", std::process::id()));
    downloader(&url, &temp_path)?;

    if !temp_path.is_file() {
        return Err(format!(
            "downloaded model temp file is missing: {}",
            temp_path.display()
        ));
    }
    validate_model_file(&temp_path, model)?;

    fs::rename(&temp_path, &location.model_path).map_err(|e| {
        format!(
            "failed to move model from {} to {}: {e}",
            temp_path.display(),
            location.model_path.display()
        )
    })?;

    write_manifest(
        &location.manifest_path,
        model,
        &location.model_path,
        &revision,
    )?;

    Ok(location.model_path)
}

fn cached_model_matches(
    location: &ModelLocation,
    model: &LocalLlmModelConfig,
    revision: &str,
) -> Result<bool, String> {
    let manifest = match read_manifest(&location.manifest_path)? {
        Some(value) => value,
        None => return Ok(false),
    };
    Ok(manifest.repo == model.repo && manifest.file == model.file && manifest.revision == revision)
}

pub fn model_download_url(repo: &str, revision: &str, file: &str) -> String {
    let base = std::env::var("DIRECLAW_LOCAL_LLM_HF_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://huggingface.co".to_string());
    format!(
        "{}/{}/resolve/{}/{}?download=true",
        base.trim_end_matches('/'),
        repo.trim_matches('/'),
        revision.trim(),
        file.trim_start_matches('/')
    )
}

fn write_manifest(
    manifest_path: &Path,
    model: &LocalLlmModelConfig,
    model_path: &Path,
    revision: &str,
) -> Result<(), String> {
    let metadata = fs::metadata(model_path).map_err(|e| {
        format!(
            "failed reading model metadata {}: {e}",
            model_path.display()
        )
    })?;
    let digest = sha256_file(model_path)?;
    let manifest = ModelManifest {
        repo: model.repo.clone(),
        file: model.file.clone(),
        revision: revision.to_string(),
        downloaded_at: now_secs(),
        size_bytes: metadata.len(),
        sha256: digest,
    };
    let body = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| format!("failed to serialize model manifest: {e}"))?;
    fs::write(manifest_path, body).map_err(|e| {
        format!(
            "failed to write model manifest {}: {e}",
            manifest_path.display()
        )
    })
}

fn read_manifest(path: &Path) -> Result<Option<ModelManifest>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let raw = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let manifest = match serde_json::from_str::<ModelManifest>(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    Ok(Some(manifest))
}

fn validate_model_file(path: &Path, model: &LocalLlmModelConfig) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|e| format!("failed to read model file metadata {}: {e}", path.display()))?;
    if metadata.len() == 0 {
        return Err(format!("model file is empty: {}", path.display()));
    }
    if let Some(expected) = model.sha256.as_ref().filter(|v| !v.trim().is_empty()) {
        let actual = sha256_file(path)?;
        if actual != expected.to_ascii_lowercase() {
            return Err(format!(
                "model sha256 mismatch for {}: expected {}, got {}",
                path.display(),
                expected,
                actual
            ));
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|e| format!("failed to open {} for sha256: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("failed to read {} for sha256: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn download_model_file(url: &str, target: &Path) -> Result<(), String> {
    let mut request = ureq::get(url);
    if let Some(token) = std::env::var("HUGGING_FACE_HUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("HF_TOKEN").ok())
        .filter(|value| !value.trim().is_empty())
    {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let response = request
        .call()
        .map_err(|e| format!("failed to download model from {url}: {e}"))?;

    let mut reader = response.into_reader();
    let mut file = fs::File::create(target)
        .map_err(|e| format!("failed to create model file {}: {e}", target.display()))?;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("failed reading model HTTP response: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("failed writing model file {}: {e}", target.display()))?;
    }
    file.flush()
        .map_err(|e| format!("failed flushing model file {}: {e}", target.display()))?;
    Ok(())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
