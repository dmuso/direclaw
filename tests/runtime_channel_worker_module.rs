use direclaw::runtime::channel_worker::tick_slack_worker;
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
