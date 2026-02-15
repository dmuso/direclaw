use direclaw::channels::slack;

#[test]
fn channels_slack_auth_module_path_exports_credential_health() {
    let settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: eng
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {}
channels:
  slack:
    enabled: true
"#,
    )
    .expect("settings");

    let _ = slack::auth::profile_credential_health(&settings);
}
