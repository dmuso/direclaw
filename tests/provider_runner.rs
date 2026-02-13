use direclaw::provider::{
    run_provider, write_file_backed_prompt, PromptArtifacts, ProviderError, ProviderKind,
    ProviderRequest, RunnerBinaries,
};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;
use tempfile::tempdir;

fn write_script(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn base_request(
    provider: ProviderKind,
    model: &str,
    cwd: &Path,
    artifacts: PromptArtifacts,
) -> ProviderRequest {
    ProviderRequest {
        agent_id: "agent-x".to_string(),
        provider,
        model: model.to_string(),
        cwd: cwd.to_path_buf(),
        message: "run".to_string(),
        prompt_artifacts: artifacts,
        timeout: Duration::from_secs(1),
        reset_requested: false,
        fresh_on_failure: false,
        env_overrides: BTreeMap::new(),
    }
}

#[test]
fn mocked_anthropic_success_and_model_mapping() {
    let dir = tempdir().expect("tempdir");
    let bin = dir.path().join("claude-mock");
    write_script(&bin, "#!/bin/sh\necho 'anthropic response'\n");

    let artifacts =
        write_file_backed_prompt(dir.path(), "req-a", "prompt", "ctx").expect("artifacts");
    let request = base_request(
        ProviderKind::Anthropic,
        "sonnet",
        dir.path(),
        artifacts.clone(),
    );
    let bins = RunnerBinaries {
        anthropic: bin.display().to_string(),
        openai: "unused".to_string(),
    };

    let result = run_provider(&request, &bins).expect("success");
    assert_eq!(result.message, "anthropic response");
    assert!(result.log.command_form.contains("claude-sonnet-4-5"));
    assert_eq!(result.log.prompt_file, artifacts.prompt_file);
    assert_eq!(result.log.context_files, artifacts.context_files);
}

#[test]
fn mocked_openai_jsonl_success() {
    let dir = tempdir().expect("tempdir");
    let bin = dir.path().join("codex-mock");
    write_script(
        &bin,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"final answer\"}}'\n",
    );

    let artifacts =
        write_file_backed_prompt(dir.path(), "req-b", "prompt", "ctx").expect("artifacts");
    let request = base_request(ProviderKind::OpenAi, "gpt-5.2", dir.path(), artifacts);
    let bins = RunnerBinaries {
        anthropic: "unused".to_string(),
        openai: bin.display().to_string(),
    };

    let result = run_provider(&request, &bins).expect("success");
    assert_eq!(result.message, "final answer");
}

#[test]
fn provider_non_zero_exit_is_explicit() {
    let dir = tempdir().expect("tempdir");
    let bin = dir.path().join("claude-fail");
    write_script(&bin, "#!/bin/sh\necho 'boom' 1>&2\nexit 17\n");

    let artifacts =
        write_file_backed_prompt(dir.path(), "req-c", "prompt", "ctx").expect("artifacts");
    let request = base_request(ProviderKind::Anthropic, "opus", dir.path(), artifacts);
    let bins = RunnerBinaries {
        anthropic: bin.display().to_string(),
        openai: "unused".to_string(),
    };

    let err = run_provider(&request, &bins).expect_err("expected failure");
    match err {
        ProviderError::NonZeroExit { exit_code, log, .. } => {
            let log = *log;
            assert_eq!(exit_code, 17);
            assert_eq!(log.agent_id, "agent-x");
            assert_eq!(log.exit_code, Some(17));
            assert!(!log.timed_out);
            assert!(log.command_form.contains("claude-fail"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn provider_timeout_is_explicit() {
    let dir = tempdir().expect("tempdir");
    let bin = dir.path().join("claude-timeout");
    write_script(&bin, "#!/bin/sh\nsleep 2\necho late\n");

    let artifacts =
        write_file_backed_prompt(dir.path(), "req-d", "prompt", "ctx").expect("artifacts");
    let mut request = base_request(ProviderKind::Anthropic, "sonnet", dir.path(), artifacts);
    request.timeout = Duration::from_millis(100);

    let bins = RunnerBinaries {
        anthropic: bin.display().to_string(),
        openai: "unused".to_string(),
    };

    let err = run_provider(&request, &bins).expect_err("expected timeout");
    match err {
        ProviderError::Timeout { log, .. } => {
            let log = *log;
            assert_eq!(log.agent_id, "agent-x");
            assert!(log.timed_out);
            assert!(log.command_form.contains("claude-timeout"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn provider_missing_binary_is_explicit() {
    let dir = tempdir().expect("tempdir");
    let artifacts =
        write_file_backed_prompt(dir.path(), "req-e", "prompt", "ctx").expect("artifacts");
    let request = base_request(ProviderKind::Anthropic, "sonnet", dir.path(), artifacts);

    let bins = RunnerBinaries {
        anthropic: dir.path().join("does-not-exist").display().to_string(),
        openai: "unused".to_string(),
    };

    let err = run_provider(&request, &bins).expect_err("expected missing binary");
    match err {
        ProviderError::MissingBinary { log, .. } => {
            let log = *log;
            assert_eq!(log.agent_id, "agent-x");
            assert_eq!(log.exit_code, None);
            assert!(!log.timed_out);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn provider_parse_failure_is_explicit_for_openai() {
    let dir = tempdir().expect("tempdir");
    let bin = dir.path().join("codex-bad");
    write_script(&bin, "#!/bin/sh\necho '{not-json}'\n");

    let artifacts =
        write_file_backed_prompt(dir.path(), "req-f", "prompt", "ctx").expect("artifacts");
    let request = base_request(ProviderKind::OpenAi, "gpt-5.2", dir.path(), artifacts);
    let bins = RunnerBinaries {
        anthropic: "unused".to_string(),
        openai: bin.display().to_string(),
    };

    let err = run_provider(&request, &bins).expect_err("expected parse failure");
    match err {
        ProviderError::ParseFailure { log, .. } => {
            let log = *log.expect("parse failure should include invocation log");
            assert_eq!(log.agent_id, "agent-x");
            assert_eq!(log.exit_code, Some(0));
            assert!(!log.timed_out);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
