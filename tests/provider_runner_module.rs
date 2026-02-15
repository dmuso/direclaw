use direclaw::provider::runner::run_provider;
use direclaw::provider::{write_file_backed_prompt, ProviderKind, ProviderRequest, RunnerBinaries};
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

#[test]
fn runner_module_executes_openai_request() {
    let dir = tempdir().expect("tempdir");
    let bin = dir.path().join("codex-mock");
    write_script(
        &bin,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"ok\"}}'\n",
    );

    let artifacts =
        write_file_backed_prompt(dir.path(), "req-runner", "prompt", "ctx").expect("artifacts");
    let request = ProviderRequest {
        agent_id: "agent-1".to_string(),
        provider: ProviderKind::OpenAi,
        model: "gpt-5.2".to_string(),
        cwd: dir.path().to_path_buf(),
        message: "run".to_string(),
        prompt_artifacts: artifacts,
        timeout: Duration::from_secs(1),
        reset_requested: false,
        fresh_on_failure: false,
        env_overrides: BTreeMap::new(),
    };
    let binaries = RunnerBinaries {
        anthropic: "unused".to_string(),
        openai: bin.display().to_string(),
    };

    let result = run_provider(&request, &binaries).expect("provider run");
    assert_eq!(result.message, "ok");
}
