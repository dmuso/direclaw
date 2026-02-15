use direclaw::orchestration::diagnostics::{persist_provider_invocation_log, provider_error_log};
use direclaw::provider::{InvocationLog, ProviderError, ProviderKind};
use std::path::Path;
use tempfile::tempdir;

fn sample_log(root: &Path) -> InvocationLog {
    InvocationLog {
        agent_id: "agent-1".to_string(),
        provider: ProviderKind::OpenAi,
        model: "gpt-5".to_string(),
        command_form: "codex exec".to_string(),
        working_directory: root.to_path_buf(),
        prompt_file: root.join("prompt.md"),
        context_files: vec![root.join("ctx.md")],
        exit_code: Some(0),
        timed_out: false,
    }
}

#[test]
fn diagnostics_module_exposes_provider_error_log_extraction() {
    let dir = tempdir().expect("tempdir");
    let log = sample_log(dir.path());
    let error = ProviderError::Timeout {
        provider: ProviderKind::OpenAi,
        timeout_ms: 1,
        log: Box::new(log.clone()),
    };

    let extracted = provider_error_log(&error).expect("timeout carries invocation log");
    assert_eq!(extracted.agent_id, log.agent_id);
    assert_eq!(extracted.provider, log.provider);
}

#[test]
fn diagnostics_module_persists_provider_invocation_log_json() {
    let dir = tempdir().expect("tempdir");
    let log = sample_log(dir.path());

    persist_provider_invocation_log(dir.path(), &log).expect("persist invocation log");

    let raw = std::fs::read_to_string(dir.path().join("provider_invocation.json"))
        .expect("read invocation log file");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("valid json payload");
    assert_eq!(parsed["agentId"], "agent-1");
    assert_eq!(parsed["provider"], "openai");
    assert_eq!(parsed["model"], "gpt-5");
    assert_eq!(parsed["timedOut"], false);
}
