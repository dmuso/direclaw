use direclaw::provider::types::{ProviderKind, ProviderRequest};
use std::collections::BTreeMap;
use std::time::Duration;

#[test]
fn provider_types_module_exposes_core_request_types() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = ProviderRequest {
        agent_id: "agent-1".to_string(),
        provider: ProviderKind::OpenAi,
        model: "gpt-5.2".to_string(),
        cwd: dir.path().to_path_buf(),
        message: "run".to_string(),
        prompt_artifacts: direclaw::provider::write_file_backed_prompt(
            dir.path(),
            "req-types",
            "prompt",
            "ctx",
        )
        .expect("prompt artifacts"),
        timeout: Duration::from_secs(1),
        reset_requested: false,
        fresh_on_failure: false,
        env_overrides: BTreeMap::new(),
    };

    assert_eq!(request.provider, ProviderKind::OpenAi);
}
