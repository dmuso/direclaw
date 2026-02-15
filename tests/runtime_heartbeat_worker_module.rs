use direclaw::runtime::heartbeat_worker::{configured_heartbeat_interval, tick_heartbeat_worker};
use std::time::Duration;

#[test]
fn runtime_heartbeat_worker_module_exposes_tick() {
    tick_heartbeat_worker().expect("heartbeat tick should be a no-op success");
}

#[test]
fn runtime_heartbeat_worker_module_exposes_configured_interval() {
    let disabled: direclaw::config::Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring:
  heartbeat_interval: 0
channels: {}
"#,
    )
    .expect("parse settings");
    assert_eq!(configured_heartbeat_interval(&disabled), None);

    let enabled: direclaw::config::Settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring:
  heartbeat_interval: 30
channels: {}
"#,
    )
    .expect("parse settings");
    assert_eq!(
        configured_heartbeat_interval(&enabled),
        Some(Duration::from_secs(30))
    );
}
