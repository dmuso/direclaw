use direclaw::app::command_handlers::workflows::cmd_workflow;

#[test]
fn workflow_handler_rejects_unknown_subcommand() {
    let err = cmd_workflow(&["bogus".to_string()]).expect_err("unknown subcommand should fail");
    assert_eq!(err, "unknown workflow subcommand `bogus`");
}
