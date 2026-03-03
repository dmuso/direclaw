use crate::local_llm::LocalLlmPromptTemplates;
use crate::local_llm::{LocalLlmInferenceConfig, LocalLlmModelConfig};
use candle::quantized::gguf_file;
use candle::{Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_qwen3::ModelWeights;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tokenizers::Tokenizer;

pub struct CandleQwenRuntime {
    model: Mutex<ModelWeights>,
    tokenizer: Tokenizer,
    device: Device,
    eos_token_id: Option<u32>,
    inference: LocalLlmInferenceConfig,
    memory_bulletin_prompt_template: String,
    thread_context_prompt_template: String,
}

impl CandleQwenRuntime {
    pub fn load(
        model_path: &Path,
        _model: &LocalLlmModelConfig,
        tokenizer_path: &Path,
        prompts: &LocalLlmPromptTemplates,
        inference: &LocalLlmInferenceConfig,
    ) -> Result<Self, String> {
        let mut file = fs::File::open(model_path)
            .map_err(|e| format!("failed to open model {}: {e}", model_path.display()))?;
        let content = gguf_file::Content::read(&mut file)
            .map_err(|e| format!("failed to read gguf metadata {}: {e}", model_path.display()))?;
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| format!("failed to load tokenizer {}: {e}", tokenizer_path.display()))?;
        let device = Device::Cpu;
        let model = ModelWeights::from_gguf(content, &mut file, &device)
            .map_err(|e| format!("failed to build qwen model from gguf: {e}"))?;
        let eos_token_id = tokenizer.get_vocab(true).get("<|im_end|>").copied();

        Ok(Self {
            model: Mutex::new(model),
            tokenizer,
            device,
            eos_token_id,
            inference: inference.clone(),
            memory_bulletin_prompt_template: prompts.memory_bulletin_preprocess.clone(),
            thread_context_prompt_template: prompts.thread_context_preprocess.clone(),
        })
    }

    pub fn generate(&self, prompt: &str) -> Result<String, String> {
        let started = Instant::now();
        let budget = Duration::from_millis(self.inference.max_generation_millis);
        let check_time = |stage: &str| -> Result<(), String> {
            if started.elapsed() > budget {
                return Err(format!(
                    "local llm generation timed out after {}ms during {stage}",
                    self.inference.max_generation_millis
                ));
            }
            Ok(())
        };
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Ok(String::new());
        }
        check_time("prompt preparation")?;

        let prompt = self.cap(prompt, self.inference.max_input_chars);
        let prompt = format!("<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n");
        let encoding = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|e| format!("failed to tokenize prompt: {e}"))?;
        let prompt_ids = encoding.get_ids();
        if prompt_ids.is_empty() {
            return Ok(String::new());
        }

        let sampling = if self.inference.temperature <= 0.0 {
            Sampling::ArgMax
        } else {
            Sampling::TopP {
                p: self.inference.top_p,
                temperature: self.inference.temperature,
            }
        };
        let mut logits_processor = LogitsProcessor::from_sampling(self.inference.seed, sampling);

        let mut model = loop {
            match self.model.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::WouldBlock) => {
                    check_time("waiting for model lock")?;
                    thread::sleep(Duration::from_millis(1));
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err("failed to acquire qwen model lock".to_string());
                }
            }
        };
        model.clear_kv_cache();
        check_time("prompt forward pass")?;

        let input = Tensor::new(prompt_ids, &self.device)
            .and_then(|tensor| tensor.unsqueeze(0))
            .map_err(|e| format!("failed to prepare prompt tensor: {e}"))?;
        let logits = model
            .forward(&input, 0)
            .and_then(|tensor| tensor.squeeze(0))
            .map_err(|e| format!("failed to run prompt forward pass: {e}"))?;

        let mut next_token = logits_processor
            .sample(&logits)
            .map_err(|e| format!("failed to sample next token: {e}"))?;

        let mut generated = Vec::with_capacity(self.inference.max_new_tokens);
        generated.push(next_token);

        for index in 0..self.inference.max_new_tokens.saturating_sub(1) {
            check_time("token generation")?;
            if self.eos_token_id == Some(next_token) {
                break;
            }
            let step_input = Tensor::new(&[next_token], &self.device)
                .and_then(|tensor| tensor.unsqueeze(0))
                .map_err(|e| format!("failed to prepare decode-step tensor: {e}"))?;
            let logits = model
                .forward(&step_input, prompt_ids.len() + index)
                .and_then(|tensor| tensor.squeeze(0))
                .map_err(|e| format!("failed to run decode-step forward pass: {e}"))?;
            next_token = logits_processor
                .sample(&logits)
                .map_err(|e| format!("failed to sample decode token: {e}"))?;
            generated.push(next_token);
        }

        let text = self
            .tokenizer
            .decode(&generated, true)
            .map_err(|e| format!("failed to decode generated tokens: {e}"))?;

        Ok(self.cap(text.trim(), self.inference.max_output_chars))
    }

    pub fn memory_bulletin_prompt_template(&self) -> &str {
        &self.memory_bulletin_prompt_template
    }

    pub fn thread_context_prompt_template(&self) -> &str {
        &self.thread_context_prompt_template
    }

    fn cap(&self, input: &str, max_chars: usize) -> String {
        if input.chars().count() <= max_chars {
            return input.to_string();
        }
        input.chars().take(max_chars).collect()
    }
}
