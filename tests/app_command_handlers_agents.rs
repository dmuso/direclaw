use direclaw::app::command_handlers::agents::cmd_orchestrator_agent;

#[test]
fn orchestrator_agent_handler_rejects_unknown_subcommand() {
    let err =
        cmd_orchestrator_agent(&["bogus".to_string()]).expect_err("unknown subcommand should fail");
    assert_eq!(err, "unknown orchestrator-agent subcommand `bogus`");
}
