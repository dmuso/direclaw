use direclaw::orchestration::step_execution::resolve_runner_binaries;

#[test]
fn step_execution_module_exposes_runner_binary_resolution_defaults() {
    let binaries = resolve_runner_binaries();
    assert_eq!(binaries.anthropic, "claude");
    assert_eq!(binaries.openai, "codex");
}
