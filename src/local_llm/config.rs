use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalLlmProvider {
    Candle,
}

fn default_enabled() -> bool {
    false
}

fn default_provider() -> LocalLlmProvider {
    LocalLlmProvider::Candle
}

fn default_repo() -> String {
    "unsloth/Qwen3.5-0.8B-GGUF".to_string()
}

fn default_file() -> String {
    "Qwen3.5-0.8B-UD-IQ2_M.gguf".to_string()
}

fn default_tokenizer_repo() -> String {
    "Qwen/Qwen3.5-0.8B".to_string()
}

fn default_tokenizer_file() -> String {
    "tokenizer.json".to_string()
}

fn default_temperature() -> f64 {
    0.2
}

fn default_top_p() -> f64 {
    0.9
}

fn default_seed() -> u64 {
    42
}

fn default_max_input_chars() -> usize {
    8_000
}

fn default_max_output_chars() -> usize {
    3_000
}

fn default_max_new_tokens() -> usize {
    256
}

fn default_max_generation_millis() -> u64 {
    1_500
}

fn default_similarity_threshold() -> f32 {
    0.88
}

fn default_task_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalLlmTasksConfig {
    #[serde(default = "default_task_enabled")]
    pub memory_bulletin_preprocess: bool,
    #[serde(default = "default_task_enabled")]
    pub thread_context_preprocess: bool,
}

impl Default for LocalLlmTasksConfig {
    fn default() -> Self {
        Self {
            memory_bulletin_preprocess: default_task_enabled(),
            thread_context_preprocess: default_task_enabled(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalLlmModelConfig {
    #[serde(default = "default_repo")]
    pub repo: String,
    #[serde(default = "default_file")]
    pub file: String,
    pub revision: Option<String>,
    pub sha256: Option<String>,
    #[serde(default = "default_tokenizer_repo")]
    pub tokenizer_repo: String,
    #[serde(default = "default_tokenizer_file")]
    pub tokenizer_file: String,
}

impl Default for LocalLlmModelConfig {
    fn default() -> Self {
        Self {
            repo: default_repo(),
            file: default_file(),
            revision: None,
            sha256: None,
            tokenizer_repo: default_tokenizer_repo(),
            tokenizer_file: default_tokenizer_file(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LocalLlmInferenceConfig {
    #[serde(default = "default_max_input_chars")]
    pub max_input_chars: usize,
    #[serde(default = "default_max_output_chars")]
    pub max_output_chars: usize,
    #[serde(default = "default_max_new_tokens")]
    pub max_new_tokens: usize,
    #[serde(default = "default_max_generation_millis")]
    pub max_generation_millis: u64,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_top_p")]
    pub top_p: f64,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_similarity_threshold")]
    pub context_dedup_similarity_threshold: f32,
    #[serde(default = "default_similarity_threshold")]
    pub bulletin_dedup_similarity_threshold: f32,
}

impl Default for LocalLlmInferenceConfig {
    fn default() -> Self {
        Self {
            max_input_chars: default_max_input_chars(),
            max_output_chars: default_max_output_chars(),
            max_new_tokens: default_max_new_tokens(),
            max_generation_millis: default_max_generation_millis(),
            temperature: default_temperature(),
            top_p: default_top_p(),
            seed: default_seed(),
            context_dedup_similarity_threshold: default_similarity_threshold(),
            bulletin_dedup_similarity_threshold: default_similarity_threshold(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalLlmConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_provider")]
    pub provider: LocalLlmProvider,
    #[serde(default)]
    pub tasks: LocalLlmTasksConfig,
    #[serde(default)]
    pub model: LocalLlmModelConfig,
    #[serde(default)]
    pub inference: LocalLlmInferenceConfig,
}

impl Default for LocalLlmConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            provider: default_provider(),
            tasks: LocalLlmTasksConfig::default(),
            model: LocalLlmModelConfig::default(),
            inference: LocalLlmInferenceConfig::default(),
        }
    }
}

impl LocalLlmConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.model.repo.trim().is_empty() {
            return Err("local_llm.model.repo must be non-empty".to_string());
        }
        if self.model.file.trim().is_empty() {
            return Err("local_llm.model.file must be non-empty".to_string());
        }
        if self.model.tokenizer_repo.trim().is_empty() {
            return Err("local_llm.model.tokenizer_repo must be non-empty".to_string());
        }
        if self.model.tokenizer_file.trim().is_empty() {
            return Err("local_llm.model.tokenizer_file must be non-empty".to_string());
        }
        if self.inference.max_input_chars == 0 {
            return Err("local_llm.inference.max_input_chars must be >= 1".to_string());
        }
        if self.inference.max_output_chars == 0 {
            return Err("local_llm.inference.max_output_chars must be >= 1".to_string());
        }
        if self.inference.max_new_tokens == 0 {
            return Err("local_llm.inference.max_new_tokens must be >= 1".to_string());
        }
        if self.inference.max_generation_millis == 0 {
            return Err("local_llm.inference.max_generation_millis must be >= 1".to_string());
        }
        if !(0.0..=1.0).contains(&self.inference.top_p) || self.inference.top_p == 0.0 {
            return Err("local_llm.inference.top_p must be in (0.0, 1.0]".to_string());
        }
        if self.inference.temperature < 0.0 {
            return Err("local_llm.inference.temperature must be >= 0.0".to_string());
        }
        for (label, value) in [
            (
                "local_llm.inference.context_dedup_similarity_threshold",
                self.inference.context_dedup_similarity_threshold,
            ),
            (
                "local_llm.inference.bulletin_dedup_similarity_threshold",
                self.inference.bulletin_dedup_similarity_threshold,
            ),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(format!("{label} must be in [0.0, 1.0]"));
            }
        }
        Ok(())
    }
}
