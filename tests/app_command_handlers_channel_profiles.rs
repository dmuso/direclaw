use direclaw::app::command_handlers::channel_profiles::cmd_channel_profile;

#[test]
fn channel_profile_handler_rejects_unknown_subcommand() {
    let err =
        cmd_channel_profile(&["bogus".to_string()]).expect_err("unknown subcommand should fail");
    assert_eq!(err, "unknown channel-profile subcommand `bogus`");
}
