use direclaw::app::command_handlers::auth::cmd_auth;

#[test]
fn auth_handler_rejects_unknown_subcommand_with_usage() {
    let err = cmd_auth(&["bogus".to_string()]).expect_err("unknown subcommand should fail");
    assert_eq!(err, "usage: auth sync");
}
