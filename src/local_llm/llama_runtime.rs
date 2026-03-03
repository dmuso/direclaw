use crate::local_llm::LocalLlmPromptTemplates;
use crate::local_llm::{LocalLlmInferenceConfig, LocalLlmModelConfig};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub struct LlamaCppRuntime {
    model_path: PathBuf,
    inference: LocalLlmInferenceConfig,
    llama_cli_binary: String,
    memory_bulletin_prompt_template: String,
    thread_context_prompt_template: String,
}

impl LlamaCppRuntime {
    pub fn load(
        model_path: &Path,
        _model: &LocalLlmModelConfig,
        prompts: &LocalLlmPromptTemplates,
        inference: &LocalLlmInferenceConfig,
    ) -> Result<Self, String> {
        let binary = std::env::var("DIRECLAW_LOCAL_LLM_LLAMA_CPP_BIN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "llama-cli".to_string());
        ensure_llama_cpp_available(&binary)?;

        Ok(Self {
            model_path: model_path.to_path_buf(),
            inference: inference.clone(),
            llama_cli_binary: binary,
            memory_bulletin_prompt_template: prompts.memory_bulletin_preprocess.clone(),
            thread_context_prompt_template: prompts.thread_context_preprocess.clone(),
        })
    }

    pub fn generate(&self, prompt: &str) -> Result<String, String> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Ok(String::new());
        }
        let prompt = self.cap(prompt, self.inference.max_input_chars);
        let output = self.run_llama_cpp(&prompt)?;
        Ok(self.cap(output.trim(), self.inference.max_output_chars))
    }

    pub fn memory_bulletin_prompt_template(&self) -> &str {
        &self.memory_bulletin_prompt_template
    }

    pub fn thread_context_prompt_template(&self) -> &str {
        &self.thread_context_prompt_template
    }

    fn run_llama_cpp(&self, prompt: &str) -> Result<String, String> {
        let mut child = Command::new(&self.llama_cli_binary)
            .arg("-m")
            .arg(&self.model_path)
            .arg("-n")
            .arg(self.inference.max_new_tokens.to_string())
            .arg("--temp")
            .arg(self.inference.temperature.to_string())
            .arg("--top-p")
            .arg(self.inference.top_p.to_string())
            .arg("--seed")
            .arg(self.inference.seed.to_string())
            .arg("--no-display-prompt")
            .arg("-p")
            .arg(prompt)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!(
                    "failed to start llama.cpp command `{}`: {e}",
                    self.llama_cli_binary
                )
            })?;

        let timeout = Duration::from_millis(self.inference.max_generation_millis);
        let started = Instant::now();
        loop {
            if started.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "local llm generation timed out after {}ms",
                    self.inference.max_generation_millis
                ));
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut stdout = Vec::new();
                    if let Some(mut handle) = child.stdout.take() {
                        handle
                            .read_to_end(&mut stdout)
                            .map_err(|e| format!("failed reading llama.cpp stdout: {e}"))?;
                    }
                    let mut stderr = Vec::new();
                    if let Some(mut handle) = child.stderr.take() {
                        handle
                            .read_to_end(&mut stderr)
                            .map_err(|e| format!("failed reading llama.cpp stderr: {e}"))?;
                    }
                    if !status.success() {
                        let stderr = String::from_utf8_lossy(&stderr);
                        return Err(format!(
                            "llama.cpp command failed with status {}: {}",
                            status,
                            stderr.trim()
                        ));
                    }
                    return Ok(String::from_utf8_lossy(&stdout).to_string());
                }
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(e) => return Err(format!("failed waiting for llama.cpp process: {e}")),
            }
        }
    }

    fn cap(&self, input: &str, max_chars: usize) -> String {
        if input.chars().count() <= max_chars {
            return input.to_string();
        }
        input.chars().take(max_chars).collect()
    }
}

fn ensure_llama_cpp_available(binary: &str) -> Result<(), String> {
    match Command::new(binary)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => Ok(()),
        Err(e) => Err(format!(
            "llama.cpp binary `{binary}` is not available; install llama.cpp and ensure `{binary}` is on PATH (or set DIRECLAW_LOCAL_LLM_LLAMA_CPP_BIN): {e}"
        )),
    }
}
