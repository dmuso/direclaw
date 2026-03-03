use direclaw::local_llm::load_local_llm_prompt_templates;
use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths, DEFAULT_STATE_ROOT_DIR};
use std::fs;
use tempfile::tempdir;

#[test]
fn runtime_state_paths_module_exposes_state_root_bootstrap_apis() {
    assert_eq!(DEFAULT_STATE_ROOT_DIR, ".direclaw");

    let tmp = tempdir().expect("tempdir");
    let state_root = tmp.path().join(".direclaw");
    let paths = StatePaths::new(&state_root);

    bootstrap_state_root(&paths).expect("bootstrap state root");

    for dir in paths.required_directories() {
        assert!(dir.is_dir(), "expected directory {}", dir.display());
    }
    assert!(
        state_root
            .join("models/prompts/memory_bulletin_preprocess.prompt.md")
            .is_file(),
        "expected local llm memory prompt to be bootstrapped"
    );
    assert!(
        state_root
            .join("models/prompts/thread_context_preprocess.prompt.md")
            .is_file(),
        "expected local llm thread prompt to be bootstrapped"
    );
    fs::write(
        state_root.join("models/prompts/thread_context_preprocess.prompt.md"),
        "custom thread prompt",
    )
    .expect("write custom prompt");
    let templates = load_local_llm_prompt_templates(&state_root);
    assert_eq!(templates.thread_context_preprocess, "custom thread prompt");
}
