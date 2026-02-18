use direclaw::config::{AuthSyncConfig, Monitoring, Settings, SettingsOrchestrator};
use direclaw::memory::MemoryConfig;
use direclaw::orchestration::slack_target::{
    parse_slack_target_ref, slack_target_ref_to_value, validate_profile_mapping, SlackPostingMode,
    SlackTargetRef,
};
use serde_json::json;
use std::collections::BTreeMap;
use tempfile::tempdir;

#[test]
fn slack_target_ref_schema_validation_rejects_missing_required_fields() {
    let err = parse_slack_target_ref(
        &json!({
            "channel": "slack",
            "channelId": "C123"
        }),
        "targetRef",
    )
    .expect_err("missing required field should fail");
    assert!(
        err.contains("targetRef.channelProfileId"),
        "unexpected error: {err}"
    );
}

#[test]
fn slack_target_ref_schema_validation_rejects_invalid_posting_mode() {
    let err = parse_slack_target_ref(
        &json!({
            "channel": "slack",
            "channelProfileId": "slack_main",
            "channelId": "C123",
            "postingMode": "invalid_mode"
        }),
        "targetRef",
    )
    .expect_err("invalid posting mode should fail");
    assert!(
        err.contains("targetRef.postingMode"),
        "unexpected error: {err}"
    );
}

#[test]
fn slack_target_ref_schema_normalization_trims_fields() {
    let parsed = parse_slack_target_ref(
        &json!({
            "channel": "slack",
            "channelProfileId": " slack_main ",
            "channelId": " C123 ",
            "threadTs": " 1700000000.1 ",
            "postingMode": "thread_reply"
        }),
        "targetRef",
    )
    .expect("valid targetRef")
    .expect("slack target");
    assert_eq!(parsed.channel_profile_id, "slack_main");
    assert_eq!(parsed.channel_id, "C123");
    assert_eq!(parsed.thread_ts.as_deref(), Some("1700000000.1"));
    assert_eq!(parsed.posting_mode, SlackPostingMode::ThreadReply);
}

#[test]
fn slack_target_ref_schema_rejects_unsupported_target_channel() {
    let err = parse_slack_target_ref(
        &json!({
            "channel": "local",
            "conversationId": "chat-1"
        }),
        "targetRef",
    )
    .expect_err("unsupported channel should fail");
    assert!(err.contains("targetRef.channel"), "unexpected error: {err}");
}

#[test]
fn profile_mapping_validation_rejects_cross_orchestrator_target() {
    let temp = tempdir().expect("tempdir");
    let settings = Settings {
        workspaces_path: temp.path().to_path_buf(),
        shared_workspaces: BTreeMap::new(),
        orchestrators: BTreeMap::from_iter([
            (
                "alpha".to_string(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            ),
            (
                "beta".to_string(),
                SettingsOrchestrator {
                    private_workspace: None,
                    shared_access: Vec::new(),
                },
            ),
        ]),
        channel_profiles: BTreeMap::from_iter([
            (
                "slack_alpha".to_string(),
                direclaw::config::ChannelProfile {
                    channel: direclaw::config::ChannelKind::Slack,
                    orchestrator_id: "alpha".to_string(),
                    slack_app_user_id: None,
                    require_mention_in_channels: None,
                },
            ),
            (
                "slack_beta".to_string(),
                direclaw::config::ChannelProfile {
                    channel: direclaw::config::ChannelKind::Slack,
                    orchestrator_id: "beta".to_string(),
                    slack_app_user_id: None,
                    require_mention_in_channels: None,
                },
            ),
        ]),
        monitoring: Monitoring::default(),
        channels: BTreeMap::new(),
        auth_sync: AuthSyncConfig::default(),
        memory: MemoryConfig::default(),
    };
    let target = SlackTargetRef {
        channel_profile_id: "slack_beta".to_string(),
        channel_id: "C42".to_string(),
        thread_ts: None,
        posting_mode: SlackPostingMode::ChannelPost,
    };
    let err = validate_profile_mapping(&settings, "alpha", Some(&target))
        .expect_err("cross-orchestrator target should fail");
    assert!(
        err.contains("orchestrator"),
        "expected orchestrator mismatch error, got {err}"
    );
    let as_value = slack_target_ref_to_value(&target);
    assert_eq!(as_value["channelProfileId"], "slack_beta");
}
