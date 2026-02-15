use direclaw::app::command_support::default_orchestrator_config;

#[test]
fn command_support_builds_default_orchestrator_config() {
    let orchestrator = default_orchestrator_config("main");
    assert_eq!(orchestrator.default_workflow, "default");
}
