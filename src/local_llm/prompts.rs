use std::fs;
use std::path::Path;

const MEMORY_BULLETIN_PREPROCESS_PROMPT: &str =
    include_str!("../prompts/assets/local_llm/memory_bulletin_preprocess.prompt.md");
const THREAD_CONTEXT_PREPROCESS_PROMPT: &str =
    include_str!("../prompts/assets/local_llm/thread_context_preprocess.prompt.md");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalLlmPromptTemplates {
    pub memory_bulletin_preprocess: String,
    pub thread_context_preprocess: String,
}

pub fn ensure_local_llm_prompt_assets(state_root: &Path) -> Result<(), String> {
    let prompts_dir = state_root.join("models/prompts");
    fs::create_dir_all(&prompts_dir)
        .map_err(|e| format!("failed to create {}: {e}", prompts_dir.display()))?;

    ensure_file(
        &prompts_dir.join("memory_bulletin_preprocess.prompt.md"),
        MEMORY_BULLETIN_PREPROCESS_PROMPT,
    )?;
    ensure_file(
        &prompts_dir.join("thread_context_preprocess.prompt.md"),
        THREAD_CONTEXT_PREPROCESS_PROMPT,
    )?;
    Ok(())
}

pub fn memory_bulletin_prompt_template() -> &'static str {
    MEMORY_BULLETIN_PREPROCESS_PROMPT
}

pub fn thread_context_prompt_template() -> &'static str {
    THREAD_CONTEXT_PREPROCESS_PROMPT
}

pub fn load_local_llm_prompt_templates(state_root: &Path) -> LocalLlmPromptTemplates {
    let prompts_dir = state_root.join("models/prompts");
    let memory = fs::read_to_string(prompts_dir.join("memory_bulletin_preprocess.prompt.md"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| MEMORY_BULLETIN_PREPROCESS_PROMPT.to_string());
    let thread = fs::read_to_string(prompts_dir.join("thread_context_preprocess.prompt.md"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| THREAD_CONTEXT_PREPROCESS_PROMPT.to_string());
    LocalLlmPromptTemplates {
        memory_bulletin_preprocess: memory,
        thread_context_preprocess: thread,
    }
}

fn ensure_file(path: &Path, body: &str) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))
}
