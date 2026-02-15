use direclaw::app::command_handlers::update::cmd_update;

#[test]
fn update_handler_rejects_unknown_subcommand_with_usage() {
    let err = cmd_update(&["bogus".to_string()]).expect_err("unknown subcommand should fail");
    assert_eq!(err, "usage: update [check|apply]");
}
