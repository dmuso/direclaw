use direclaw::runtime::channel_worker::{tick_slack_worker, PollingDefaults};
use tempfile::tempdir;

#[test]
fn runtime_channel_worker_module_exposes_slack_tick() {
    let dir = tempdir().expect("tempdir");
    let settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
"#,
    )
    .expect("parse settings");

    let err = tick_slack_worker(dir.path(), &settings).expect_err("slack should be disabled");
    assert!(err.contains("slack channel is disabled"));
}

#[test]
fn runtime_channel_worker_module_exposes_polling_defaults() {
    let defaults = PollingDefaults::default();
    assert_eq!(defaults.queue_poll_interval_secs, 1);
    assert_eq!(defaults.outbound_poll_interval_secs, 1);
}
