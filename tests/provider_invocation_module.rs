use direclaw::provider::invocation::build_invocation;
use direclaw::provider::{ProviderKind, ProviderRequest, RunnerBinaries};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

fn sample_request(provider: ProviderKind, cwd: &Path) -> ProviderRequest {
    ProviderRequest {
        agent_id: "agent-1".to_string(),
        provider,
        model: "sonnet".to_string(),
        cwd: cwd.to_path_buf(),
        message: "use files".to_string(),
        prompt_artifacts: direclaw::provider::write_file_backed_prompt(
            cwd, "req-1", "prompt", "ctx",
        )
        .expect("prompt artifacts"),
        timeout: Duration::from_secs(1),
        reset_requested: false,
        fresh_on_failure: false,
        env_overrides: BTreeMap::new(),
    }
}

#[test]
fn invocation_module_builds_openai_resume_args() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut req = sample_request(ProviderKind::OpenAi, dir.path());
    req.model = "gpt-5.2".to_string();

    let spec = build_invocation(&req, &RunnerBinaries::default()).expect("build");
    assert_eq!(spec.binary, "codex");
    assert!(spec.args.contains(&"resume".to_string()));
    assert!(spec.args.contains(&"--full-auto".to_string()));
    assert!(!spec.args.contains(&"--sandbox".to_string()));
    assert!(!spec.args.contains(&"workspace-write".to_string()));
}
