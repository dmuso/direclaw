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
