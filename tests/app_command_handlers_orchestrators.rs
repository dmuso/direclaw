use direclaw::app::command_handlers::orchestrators::cmd_orchestrator;

#[test]
fn orchestrator_handler_rejects_unknown_subcommand() {
    let err = cmd_orchestrator(&["bogus".to_string()]).expect_err("unknown subcommand should fail");
    assert_eq!(err, "unknown orchestrator subcommand `bogus`");
}
