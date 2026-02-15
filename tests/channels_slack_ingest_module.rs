use direclaw::channels::slack::ingest::should_accept_channel_message;
use direclaw::config::{ChannelKind, ChannelProfile};
use std::collections::BTreeSet;

#[test]
fn channels_slack_ingest_module_path_exports_filtering_helper() {
    let profile = ChannelProfile {
        channel: ChannelKind::Slack,
        orchestrator_id: "eng".to_string(),
        slack_app_user_id: Some("U123".to_string()),
        require_mention_in_channels: Some(true),
    };
    let allowlist = BTreeSet::new();
    assert!(should_accept_channel_message(
        &profile,
        &allowlist,
        "C123",
        "hello <@U123>",
        "1700000000.1",
        None
    ));
}
