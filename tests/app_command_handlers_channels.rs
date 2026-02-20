use direclaw::app::command_handlers::channels::{cmd_channels, cmd_send};
use direclaw::queue::IncomingMessage;
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn channels_handler_module_exposes_commands() {
    let _ = cmd_channels as fn(&[String]) -> Result<String, String>;
    let _ = cmd_send as fn(&[String]) -> Result<String, String>;
}

#[test]
fn send_enqueues_direct_message_for_user_intent() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let old_home = std::env::var_os("HOME");
    std::env::set_var("HOME", temp.path());

    let state_root = temp.path().join(".direclaw");
    fs::create_dir_all(&state_root).expect("create state root");
    let workspace_root = temp.path().join("workspaces");
    let orchestrator_workspace = workspace_root.join("main");
    fs::create_dir_all(&orchestrator_workspace).expect("orchestrator workspace");

    fs::write(
        state_root.join("config.yaml"),
        format!(
            r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  local-default:
    channel: local
    orchestrator_id: main
monitoring: {{}}
channels: {{}}
"#,
            workspace_root.display(),
            orchestrator_workspace.display()
        ),
    )
    .expect("write settings");

    let queued =
        cmd_send(&["local-default".to_string(), "hello".to_string()]).expect("queue message");
    let message_id = queued
        .lines()
        .find_map(|line| line.strip_prefix("message_id="))
        .expect("message id line")
        .to_string();

    let incoming_path = orchestrator_workspace
        .join("queue/incoming")
        .join(format!("{message_id}.json"));
    let raw = fs::read_to_string(&incoming_path).expect("incoming payload");
    let payload: IncomingMessage = serde_json::from_str(&raw).expect("parse incoming");
    assert!(payload.is_direct);

    if let Some(value) = old_home {
        std::env::set_var("HOME", value);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn channels_slack_socket_commands_are_supported() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let old_home = std::env::var_os("HOME");
    std::env::set_var("HOME", temp.path());

    let state_root = temp.path().join(".direclaw");
    let workspace_root = temp.path().join("workspaces");
    fs::create_dir_all(&state_root).expect("create state root");
    fs::create_dir_all(&workspace_root).expect("workspace");
    fs::write(
        state_root.join("config.yaml"),
        format!(
            r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: null
    shared_access: []
channel_profiles:
  slack_main:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
monitoring: {{}}
channels:
  slack:
    enabled: true
    inbound_mode: socket
"#,
            workspace_root.display()
        ),
    )
    .expect("write settings");

    let status = cmd_channels(&[
        "slack".to_string(),
        "socket".to_string(),
        "status".to_string(),
    ])
    .expect("socket status");
    assert!(status.contains("connected="));

    let reconnect = cmd_channels(&[
        "slack".to_string(),
        "socket".to_string(),
        "reconnect".to_string(),
    ])
    .expect("socket reconnect");
    assert!(reconnect.contains("reconnect_requested=true"));

    if let Some(value) = old_home {
        std::env::set_var("HOME", value);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn channels_slack_backfill_run_command_is_supported() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let old_home = std::env::var_os("HOME");
    std::env::set_var("HOME", temp.path());

    let state_root = temp.path().join(".direclaw");
    let workspace_root = temp.path().join("workspaces");
    fs::create_dir_all(&state_root).expect("create state root");
    fs::create_dir_all(&workspace_root).expect("workspace");
    fs::write(
        state_root.join("config.yaml"),
        format!(
            r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: null
    shared_access: []
channel_profiles:
  slack_main:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
monitoring: {{}}
channels:
  slack:
    enabled: true
    inbound_mode: hybrid
    history_backfill_enabled: false
"#,
            workspace_root.display()
        ),
    )
    .expect("write settings");
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");

    let output = cmd_channels(&[
        "slack".to_string(),
        "backfill".to_string(),
        "run".to_string(),
    ])
    .expect("backfill run");
    assert!(output.contains("profiles_processed="));
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");

    if let Some(value) = old_home {
        std::env::set_var("HOME", value);
    } else {
        std::env::remove_var("HOME");
    }
}
