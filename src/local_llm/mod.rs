mod candle_runtime;
mod config;
mod model_store;
mod preprocess;
mod prompts;

pub use candle_runtime::CandleQwenRuntime;
pub use config::{
    LocalLlmConfig, LocalLlmInferenceConfig, LocalLlmModelConfig, LocalLlmProvider,
    LocalLlmTasksConfig,
};
pub use model_store::{
    ensure_model_available, ensure_model_available_with_downloader, ensure_tokenizer_available,
    model_download_url, resolve_model_location, ModelLocation,
};
pub use preprocess::{
    preprocess_memory_bulletin, preprocess_thread_context, BulletinPreprocessOutput,
};
pub use prompts::{
    ensure_local_llm_prompt_assets, load_local_llm_prompt_templates,
    memory_bulletin_prompt_template, thread_context_prompt_template, LocalLlmPromptTemplates,
};

use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Debug, Clone, PartialEq)]
struct RuntimeKey {
    state_root: String,
    model_repo: String,
    model_file: String,
    model_revision: Option<String>,
    tokenizer_repo: String,
    tokenizer_file: String,
    inference: LocalLlmInferenceConfig,
}

struct RuntimeSlot {
    key: RuntimeKey,
    runtime: Arc<CandleQwenRuntime>,
}

static LOCAL_QWEN_RUNTIME: LazyLock<Mutex<Option<RuntimeSlot>>> =
    LazyLock::new(|| Mutex::new(None));

fn runtime_key(state_root: &Path, config: &LocalLlmConfig) -> RuntimeKey {
    RuntimeKey {
        state_root: state_root.display().to_string(),
        model_repo: config.model.repo.clone(),
        model_file: config.model.file.clone(),
        model_revision: config.model.revision.clone(),
        tokenizer_repo: config.model.tokenizer_repo.clone(),
        tokenizer_file: config.model.tokenizer_file.clone(),
        inference: config.inference.clone(),
    }
}

fn should_reload_runtime(current: Option<&RuntimeKey>, next: &RuntimeKey) -> bool {
    current.map(|value| *value != *next).unwrap_or(true)
}

pub fn initialize_local_runtime(state_root: &Path, config: &LocalLlmConfig) -> Result<(), String> {
    initialize_local_runtime_with_loader(state_root, config, |state_root, config| {
        let model_path = ensure_model_available(state_root, &config.model)?;
        let tokenizer_path = ensure_tokenizer_available(state_root, &config.model)?;
        let prompts = load_local_llm_prompt_templates(state_root);
        CandleQwenRuntime::load(
            &model_path,
            &config.model,
            &tokenizer_path,
            &prompts,
            &config.inference,
        )
    })
}

fn initialize_local_runtime_with_loader<F>(
    state_root: &Path,
    config: &LocalLlmConfig,
    loader: F,
) -> Result<(), String>
where
    F: FnOnce(&Path, &LocalLlmConfig) -> Result<CandleQwenRuntime, String>,
{
    if !config.enabled {
        return Ok(());
    }
    let key = runtime_key(state_root, config);
    let mut slot = LOCAL_QWEN_RUNTIME
        .lock()
        .map_err(|_| "failed to acquire local runtime lock".to_string())?;
    if !should_reload_runtime(slot.as_ref().map(|value| &value.key), &key) {
        return Ok(());
    }
    let runtime = loader(state_root, config)?;
    *slot = Some(RuntimeSlot {
        key,
        runtime: Arc::new(runtime),
    });
    Ok(())
}

pub fn local_runtime() -> Option<Arc<CandleQwenRuntime>> {
    let slot = LOCAL_QWEN_RUNTIME.lock().ok()?;
    slot.as_ref().map(|current| current.runtime.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_reload_key_logic_skips_only_when_keys_match() {
        let state_root = Path::new("/tmp/direclaw-tests");
        let config = LocalLlmConfig {
            enabled: true,
            ..LocalLlmConfig::default()
        };
        let current_key = runtime_key(state_root, &config);
        assert!(
            !should_reload_runtime(Some(&current_key), &current_key),
            "matching key should not trigger reload"
        );

        let mut changed_config = config.clone();
        changed_config.model.revision = Some("rev-2".to_string());
        let changed_key = runtime_key(state_root, &changed_config);
        assert!(
            should_reload_runtime(Some(&current_key), &changed_key),
            "changed key should trigger reload"
        );
    }
}
