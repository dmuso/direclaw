pub mod invocation;
pub mod model_map;
pub mod output_parse;
pub mod prompt_files;
pub mod runner;
pub mod types;

pub use invocation::build_invocation;
pub use model_map::resolve_anthropic_model;
pub use output_parse::parse_openai_jsonl;
pub use prompt_files::{
    consume_reset_flag, read_to_string, write_file_backed_prompt, ResetResolution,
};
pub use runner::{run_provider, RunnerBinaries};
pub(crate) use types::io_error;
pub use types::{
    InvocationLog, InvocationSpec, PromptArtifacts, ProviderError, ProviderKind, ProviderRequest,
    ProviderResult,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::tempdir;

    fn sample_prompt_artifacts(base: &Path) -> PromptArtifacts {
        write_file_backed_prompt(base, "req-1", "prompt", "ctx").expect("prompt artifacts")
    }

    fn sample_request(provider: ProviderKind, base: &Path) -> ProviderRequest {
        ProviderRequest {
            agent_id: "agent-1".to_string(),
            provider,
            model: "sonnet".to_string(),
            cwd: base.to_path_buf(),
            message: "use files".to_string(),
            prompt_artifacts: sample_prompt_artifacts(base),
            timeout: Duration::from_secs(1),
            reset_requested: false,
            fresh_on_failure: false,
            env_overrides: BTreeMap::new(),
        }
    }

    #[test]
    fn anthropic_model_aliases_map() {
        assert_eq!(
            resolve_anthropic_model("sonnet").expect("map"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            resolve_anthropic_model("opus").expect("map"),
            "claude-opus-4-6"
        );
        assert!(resolve_anthropic_model("haiku").is_err());
    }

    #[test]
    fn invocation_builds_expected_anthropic_args() {
        let dir = tempdir().expect("tempdir");
        let req = sample_request(ProviderKind::Anthropic, dir.path());
        let spec = build_invocation(&req, &RunnerBinaries::default()).expect("build");
        assert_eq!(spec.binary, "claude");
        assert!(spec
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));
        assert!(spec.args.contains(&"-c".to_string()));
        assert!(spec.args.contains(&"-p".to_string()));
    }

    #[test]
    fn invocation_builds_expected_openai_args_and_resume_behavior() {
        let dir = tempdir().expect("tempdir");
        let mut req = sample_request(ProviderKind::OpenAi, dir.path());
        req.model = "gpt-5.2".to_string();

        let spec = build_invocation(&req, &RunnerBinaries::default()).expect("build");
        assert_eq!(spec.binary, "codex");
        assert_eq!(&spec.args[0], "exec");
        assert!(spec.args.contains(&"resume".to_string()));
        assert!(spec.args.contains(&"--json".to_string()));

        req.reset_requested = true;
        let reset_spec = build_invocation(&req, &RunnerBinaries::default()).expect("build reset");
        assert!(!reset_spec.args.contains(&"resume".to_string()));
    }

    #[test]
    fn openai_jsonl_parser_reads_last_completed_agent_message() {
        let data = r#"
{"type":"item.completed","item":{"type":"agent_message","text":"first"}}
{"type":"item.completed","item":{"type":"agent_message","content":[{"text":"second"}]}}
"#;

        let parsed = parse_openai_jsonl(data).expect("parsed");
        assert_eq!(parsed, "second");
    }

    #[test]
    fn reset_flags_are_consumed_once() {
        let dir = tempdir().expect("tempdir");
        let agent = dir.path().join("agent/reset_flag");
        fs::create_dir_all(agent.parent().expect("parent")).expect("create parent");
        fs::write(&agent, "1").expect("write agent");

        let first = consume_reset_flag(&agent).expect("consume");
        assert!(first.reset_requested);
        assert!(first.consumed_agent);
        assert!(!agent.exists());

        let second = consume_reset_flag(&agent).expect("consume again");
        assert!(!second.reset_requested);
    }
}
