use direclaw::orchestration::routing::{
    process_queued_message, process_queued_message_with_runner_binaries, resolve_status_run_id,
    FunctionRegistry, StatusResolutionInput,
};
use direclaw::orchestration::transitions::RoutedSelectorAction;
use direclaw::orchestration::{conversation_context, conversation_context::ThreadContextLimits};
use direclaw::provider::RunnerBinaries;
use direclaw::queue::IncomingMessage;
use direclaw::runtime::bootstrap_memory_runtime_paths;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use tempfile::tempdir;

#[test]
fn routing_module_exposes_status_resolution_and_function_catalog() {
    let input = StatusResolutionInput {
        explicit_run_id: None,
        inbound_workflow_run_id: Some("run-inbound".to_string()),
        channel_profile_id: Some("engineering".to_string()),
        conversation_id: Some("thread-1".to_string()),
    };
    let active = BTreeMap::from_iter([(
        ("engineering".to_string(), "thread-1".to_string()),
        "run-active".to_string(),
    )]);

    let resolved = resolve_status_run_id(&input, &active);
    assert_eq!(resolved.as_deref(), Some("run-inbound"));

    let ids = FunctionRegistry::new(vec![
        "workflow.status".to_string(),
        "orchestrator.list".to_string(),
    ])
    .available_function_ids();
    assert!(ids.iter().any(|id| id == "workflow.status"));
    assert!(ids.iter().any(|id| id == "orchestrator.list"));
}

#[test]
fn routing_module_process_queued_message_exposes_entrypoint() {
    let settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators: {}
channel_profiles: {}
monitoring: {}
channels: {}
"#,
    )
    .expect("settings");

    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: None,
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "status".to_string(),
        timestamp: 1,
        message_id: "m1".to_string(),
        conversation_id: Some("c1".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let state = tempdir().expect("tempdir");
    let err = process_queued_message(
        state.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::new(Vec::new()),
        |_attempt, _request, _orchestrator| None,
    )
    .expect_err("missing channel profile should fail");
    assert!(err.to_string().contains("missing `channelProfileId`"));
}

fn write_script(path: &std::path::Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn write_minimal_orchestrator_yaml(private_workspace: &std::path::Path) {
    fs::write(
        private_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: default
default_workflow: quick_answer
selection_max_retries: 2
selector_timeout_seconds: 30
agents:
  default:
    provider: openai
    model: gpt-5.3-codex
    can_orchestrate_workflows: true
workflows:
  - id: quick_answer
    version: 1
    description: quick answer
    tags: [quick]
    inputs: []
    steps:
      - id: answer
        type: agent_task
        agent: default
        prompt: answer directly
        outputs: [summary, artifact]
        output_files:
          summary: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("write orchestrator yaml");
}

#[test]
fn reply_policy_non_direct_non_mentioned_allows_selector_no_response() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
    thread_response_mode: selective_reply
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");
    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "This is context only".to_string(),
        timestamp: 1,
        message_id: "msg-no-response".to_string(),
        conversation_id: Some("C1:100.1".to_string()),
        is_direct: false,
        is_thread_reply: true,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let action = process_queued_message(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(
                private_workspace.join(".direclaw"),
            ),
            &settings,
        ),
        |_attempt, request, _orchestrator| {
            Some(format!(
                r#"{{
                  "selectorId":"{}",
                  "status":"selected",
                  "action":"no_response",
                  "reason":"low_value_thread_noise"
                }}"#,
                request.selector_id
            ))
        },
    )
    .expect("route inbound message");

    assert!(matches!(action, RoutedSelectorAction::NoResponse { .. }));
}

#[test]
fn reply_policy_explicit_mention_always_replies() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
    thread_response_mode: selective_reply
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");
    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "<@UAPP> please respond".to_string(),
        timestamp: 1,
        message_id: "msg-mention".to_string(),
        conversation_id: Some("C1:100.2".to_string()),
        is_direct: false,
        is_thread_reply: true,
        is_mentioned: true,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let codex_mock = temp.path().join("codex-mock");
    write_script(
        &codex_mock,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );
    let claude_mock = temp.path().join("claude-mock");
    write_script(
        &claude_mock,
        "#!/bin/sh\necho '[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"ok\"}[/workflow_result]'\n",
    );
    let binaries = RunnerBinaries {
        anthropic: claude_mock.display().to_string(),
        openai: codex_mock.display().to_string(),
    };

    let action = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(
                private_workspace.join(".direclaw"),
            ),
            &settings,
        ),
        Some(binaries),
        |_attempt, request, _orchestrator| {
            Some(format!(
                r#"{{
                  "selectorId":"{}",
                  "status":"selected",
                  "action":"no_response",
                  "reason":"low_value_thread_noise"
                }}"#,
                request.selector_id
            ))
        },
    )
    .expect("route inbound message");

    assert!(matches!(action, RoutedSelectorAction::WorkflowStart { .. }));
}

#[test]
fn reply_policy_direct_message_always_replies() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
    thread_response_mode: selective_reply
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");
    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "please respond".to_string(),
        timestamp: 1,
        message_id: "msg-direct".to_string(),
        conversation_id: Some("D1:100.2".to_string()),
        is_direct: true,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let codex_mock = temp.path().join("codex-mock");
    write_script(
        &codex_mock,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );
    let claude_mock = temp.path().join("claude-mock");
    write_script(
        &claude_mock,
        "#!/bin/sh\necho '[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"ok\"}[/workflow_result]'\n",
    );
    let binaries = RunnerBinaries {
        anthropic: claude_mock.display().to_string(),
        openai: codex_mock.display().to_string(),
    };

    let action = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(
                private_workspace.join(".direclaw"),
            ),
            &settings,
        ),
        Some(binaries),
        |_attempt, request, _orchestrator| {
            Some(format!(
                r#"{{
                  "selectorId":"{}",
                  "status":"selected",
                  "action":"no_response",
                  "reason":"low_value_thread_noise"
                }}"#,
                request.selector_id
            ))
        },
    )
    .expect("route inbound message");

    assert!(matches!(action, RoutedSelectorAction::WorkflowStart { .. }));
}

#[test]
fn routing_outcome_is_independent_of_channel_name() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
  local-main:
    channel: local
    orchestrator_id: main
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");

    let make_action = |inbound: IncomingMessage| {
        process_queued_message(
            temp.path(),
            &settings,
            &inbound,
            1,
            &BTreeMap::new(),
            &FunctionRegistry::v1_defaults(
                direclaw::orchestration::run_store::WorkflowRunStore::new(
                    private_workspace.join(".direclaw"),
                ),
                &settings,
            ),
            |_attempt, request, _orchestrator| {
                Some(format!(
                    r#"{{
                      "selectorId":"{}",
                      "status":"selected",
                      "action":"no_response",
                      "reason":"low_value_context"
                    }}"#,
                    request.selector_id
                ))
            },
        )
        .expect("route inbound message")
    };

    let slack_action = make_action(IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "context only".to_string(),
        timestamp: 1,
        message_id: "msg-slack".to_string(),
        conversation_id: Some("C1:100.1".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    });
    let local_action = make_action(IncomingMessage {
        channel: "local".to_string(),
        channel_profile_id: Some("local-main".to_string()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: "context only".to_string(),
        timestamp: 1,
        message_id: "msg-local".to_string(),
        conversation_id: Some("chat-1".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    });

    assert!(matches!(
        slack_action,
        RoutedSelectorAction::NoResponse { .. }
    ));
    assert!(matches!(
        local_action,
        RoutedSelectorAction::NoResponse { .. }
    ));
}

#[test]
fn local_selective_reply_can_route_no_response_for_non_mentions() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  local-main:
    channel: local
    orchestrator_id: main
    thread_response_mode: selective_reply
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");
    let inbound = IncomingMessage {
        channel: "local".to_string(),
        channel_profile_id: Some("local-main".to_string()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: "This is context only".to_string(),
        timestamp: 1,
        message_id: "msg-local-no-response".to_string(),
        conversation_id: Some("chat-1".to_string()),
        is_direct: false,
        is_thread_reply: true,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let action = process_queued_message(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(
                private_workspace.join(".direclaw"),
            ),
            &settings,
        ),
        |_attempt, request, _orchestrator| {
            Some(format!(
                r#"{{
                  "selectorId":"{}",
                  "status":"selected",
                  "action":"no_response",
                  "reason":"low_value_thread_noise"
                }}"#,
                request.selector_id
            ))
        },
    )
    .expect("route inbound message");

    assert!(matches!(action, RoutedSelectorAction::NoResponse { .. }));
}

#[test]
fn local_selector_no_response_is_overridden_when_message_is_mentioned() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  local-main:
    channel: local
    orchestrator_id: main
    thread_response_mode: selective_reply
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");
    let inbound = IncomingMessage {
        channel: "local".to_string(),
        channel_profile_id: Some("local-main".to_string()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: "agent please respond".to_string(),
        timestamp: 1,
        message_id: "msg-local-mention".to_string(),
        conversation_id: Some("chat-1".to_string()),
        is_direct: false,
        is_thread_reply: true,
        is_mentioned: true,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let codex_mock = temp.path().join("codex-mock");
    write_script(
        &codex_mock,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );
    let claude_mock = temp.path().join("claude-mock");
    write_script(
        &claude_mock,
        "#!/bin/sh\necho '[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"ok\"}[/workflow_result]'\n",
    );
    let binaries = RunnerBinaries {
        anthropic: claude_mock.display().to_string(),
        openai: codex_mock.display().to_string(),
    };

    let action = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(
                private_workspace.join(".direclaw"),
            ),
            &settings,
        ),
        Some(binaries),
        |_attempt, request, _orchestrator| {
            Some(format!(
                r#"{{
                  "selectorId":"{}",
                  "status":"selected",
                  "action":"no_response",
                  "reason":"low_value_thread_noise"
                }}"#,
                request.selector_id
            ))
        },
    )
    .expect("route inbound message");

    assert!(matches!(action, RoutedSelectorAction::WorkflowStart { .. }));
}

#[test]
fn non_fast_path_retries_selector_attempts_then_falls_back_to_default_workflow() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    fs::write(
        private_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: default
default_workflow: quick_answer
selection_max_retries: 2
selector_timeout_seconds: 30
agents:
  default:
    provider: openai
    model: gpt-5.3-codex
    can_orchestrate_workflows: true
workflows:
  - id: quick_answer
    version: 1
    description: quick answer
    tags: [quick]
    inputs: []
    steps:
      - id: answer
        type: agent_task
        agent: default
        prompt: answer directly
        outputs: [summary, artifact]
        output_files:
          summary: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("write orchestrator yaml");

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
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
        private_workspace.display()
    ))
    .expect("settings");

    let codex_mock = temp.path().join("codex-mock");
    write_script(
        &codex_mock,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );
    let claude_mock = temp.path().join("claude-mock");
    write_script(
        &claude_mock,
        "#!/bin/sh\necho '[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"ok\"}[/workflow_result]'\n",
    );
    let binaries = RunnerBinaries {
        anthropic: claude_mock.display().to_string(),
        openai: codex_mock.display().to_string(),
    };

    let inbound = IncomingMessage {
        channel: "local".to_string(),
        channel_profile_id: Some("local-default".to_string()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: "what capabilities do you currently expose?".to_string(),
        timestamp: 1,
        message_id: "msg-1".to_string(),
        conversation_id: Some("chat-1".to_string()),
        is_direct: true,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let calls = AtomicUsize::new(0);
    let action = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(
                private_workspace.join(".direclaw"),
            ),
            &settings,
        ),
        Some(binaries),
        |_attempt, _request, _orchestrator| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some("{}".to_string())
        },
    )
    .expect("route inbound message");

    assert_eq!(calls.load(Ordering::SeqCst), 3);
    match action {
        RoutedSelectorAction::WorkflowStart { workflow_id, .. } => {
            assert_eq!(workflow_id, "quick_answer");
        }
        other => panic!("expected workflow start route, got {other:?}"),
    }
}

#[test]
fn opportunistic_slack_selector_exhaustion_routes_no_response() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
    thread_response_mode: selective_reply
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");

    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "This is context only".to_string(),
        timestamp: 1,
        message_id: "msg-opportunistic-no-response".to_string(),
        conversation_id: Some("C1:100.1".to_string()),
        is_direct: false,
        is_thread_reply: true,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let calls = AtomicUsize::new(0);
    let action = process_queued_message(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(
                private_workspace.join(".direclaw"),
            ),
            &settings,
        ),
        |_attempt, _request, _orchestrator| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some("{}".to_string())
        },
    )
    .expect("route inbound message");

    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert!(matches!(action, RoutedSelectorAction::NoResponse { .. }));
}

#[test]
fn opportunistic_slack_selector_exhaustion_does_not_sleep_between_attempts() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
    thread_response_mode: selective_reply
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");

    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "context only".to_string(),
        timestamp: 1,
        message_id: "msg-opportunistic-no-delay".to_string(),
        conversation_id: Some("C1:100.1".to_string()),
        is_direct: false,
        is_thread_reply: true,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let binaries = RunnerBinaries {
        anthropic: "/path/that/does/not/exist-anthropic".to_string(),
        openai: "/path/that/does/not/exist-openai".to_string(),
    };

    let started = Instant::now();
    let action = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(
                private_workspace.join(".direclaw"),
            ),
            &settings,
        ),
        Some(binaries),
        |_attempt, _request, _orchestrator| None,
    )
    .expect("route inbound message");

    assert!(
        started.elapsed().as_secs_f32() < 2.0,
        "selector retries should not sleep"
    );
    assert!(matches!(action, RoutedSelectorAction::NoResponse { .. }));
}

#[test]
fn non_fast_path_honors_valid_selector_selected_workflow() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    fs::write(
        private_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: default
default_workflow: quick_answer
selection_max_retries: 2
selector_timeout_seconds: 30
agents:
  default:
    provider: openai
    model: gpt-5.3-codex
    can_orchestrate_workflows: true
workflows:
  - id: quick_answer
    version: 1
    description: quick answer
    tags: [quick]
    inputs: []
    steps:
      - id: answer
        type: agent_task
        agent: default
        prompt: answer directly
        outputs: [summary, artifact]
        output_files:
          summary: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}.txt
  - id: feature_delivery
    version: 1
    description: feature delivery
    tags: [delivery]
    inputs: []
    steps:
      - id: deliver
        type: agent_task
        agent: default
        prompt: deliver feature
        outputs: [summary, artifact]
        output_files:
          summary: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("write orchestrator yaml");

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: main
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");

    let codex_mock = temp.path().join("codex-mock");
    write_script(
        &codex_mock,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );
    let claude_mock = temp.path().join("claude-mock");
    write_script(
        &claude_mock,
        "#!/bin/sh\necho '[workflow_result]{\"status\":\"complete\",\"summary\":\"ok\",\"artifact\":\"ok\"}[/workflow_result]'\n",
    );
    let binaries = RunnerBinaries {
        anthropic: claude_mock.display().to_string(),
        openai: codex_mock.display().to_string(),
    };

    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: "what capabilities do you currently expose?".to_string(),
        timestamp: 1,
        message_id: "msg-2".to_string(),
        conversation_id: Some("chat-2".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let calls = AtomicUsize::new(0);
    let action = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(
                private_workspace.join(".direclaw"),
            ),
            &settings,
        ),
        Some(binaries),
        |_attempt, request, _orchestrator| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(format!(
                r#"{{
                  "selectorId":"{}",
                  "status":"selected",
                  "action":"workflow_start",
                  "selectedWorkflow":"feature_delivery"
                }}"#,
                request.selector_id
            ))
        },
    )
    .expect("route inbound message");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    match action {
        RoutedSelectorAction::WorkflowStart { workflow_id, .. } => {
            assert_eq!(workflow_id, "feature_delivery");
        }
        other => panic!("expected workflow start route, got {other:?}"),
    }
}

#[test]
fn workflow_bound_status_routes_via_selector() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: main
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");
    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "/status".to_string(),
        timestamp: 1,
        message_id: "m-status-fast".to_string(),
        conversation_id: Some("c1".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: Some("run-missing".to_string()),
        workflow_step_id: None,
    };

    let calls = AtomicUsize::new(0);
    let action = process_queued_message(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::new(Vec::new()),
        |_attempt, request, _orchestrator| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(format!(
                r#"{{
                  "selectorId":"{}",
                  "status":"selected",
                  "action":"workflow_status"
                }}"#,
                request.selector_id
            ))
        },
    )
    .expect("status route");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    match action {
        RoutedSelectorAction::WorkflowStatus {
            run_id, message, ..
        } => {
            assert_eq!(run_id.as_deref(), Some("run-missing"));
            assert!(message.contains("was not found"));
        }
        other => panic!("expected workflow status route, got {other:?}"),
    }
}

#[test]
fn routing_memory_failure_still_routes_and_persists_bulletin_payload_fields() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    fs::write(
        private_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: default
default_workflow: quick_answer
selection_max_retries: 2
selector_timeout_seconds: 30
agents:
  default:
    provider: openai
    model: gpt-5.3-codex
    can_orchestrate_workflows: true
workflows:
  - id: quick_answer
    version: 1
    description: quick answer
    tags: [quick]
    inputs: []
    steps:
      - id: answer
        type: agent_task
        agent: default
        prompt: answer directly
        outputs: [summary, artifact]
        output_files:
          summary: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("write orchestrator yaml");

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
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
memory:
  enabled: true
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");

    let runtime_root = settings
        .resolve_channel_profile_runtime_root("local-default")
        .expect("runtime root");
    fs::create_dir_all(&runtime_root).expect("runtime");
    fs::write(runtime_root.join("memory"), "block memory dir").expect("write blocker");

    let codex_mock = temp.path().join("codex-mock");
    write_script(
        &codex_mock,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );
    let claude_mock = temp.path().join("claude-mock");
    write_script(&claude_mock, "#!/bin/sh\necho 'ok'\n");
    let binaries = RunnerBinaries {
        anthropic: claude_mock.display().to_string(),
        openai: codex_mock.display().to_string(),
    };

    let inbound = IncomingMessage {
        channel: "local".to_string(),
        channel_profile_id: Some("local-default".to_string()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: "what capabilities do you currently expose?".to_string(),
        timestamp: 1,
        message_id: "msg-mem-1".to_string(),
        conversation_id: Some("chat-1".to_string()),
        is_direct: true,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let action = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(runtime_root.clone()),
            &settings,
        ),
        Some(binaries),
        |_attempt, _request, _orchestrator| None,
    )
    .expect("route inbound message");
    assert!(matches!(action, RoutedSelectorAction::WorkflowStart { .. }));

    let request_path =
        runtime_root.join("orchestrator/artifacts/selector-processing-sel-msg-mem-1.json");
    let request_json: Value =
        serde_json::from_str(&fs::read_to_string(request_path).expect("read selector request"))
            .expect("parse selector request");
    assert!(
        request_json
            .get("memoryBulletin")
            .and_then(Value::as_str)
            .is_some(),
        "expected memoryBulletin string payload"
    );
    assert!(
        request_json
            .get("memoryBulletinCitations")
            .and_then(Value::as_array)
            .is_some(),
        "expected memoryBulletinCitations array payload"
    );
}

#[test]
fn routing_persists_transcript_and_workflow_output_memories() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    fs::write(
        private_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: default
default_workflow: quick_answer
selection_max_retries: 2
selector_timeout_seconds: 30
agents:
  default:
    provider: openai
    model: gpt-5.3-codex
    can_orchestrate_workflows: true
workflows:
  - id: quick_answer
    version: 1
    description: quick answer
    tags: [quick]
    inputs: []
    steps:
      - id: answer
        type: agent_task
        agent: default
        prompt: answer directly
        outputs: [summary, decision, todo]
        output_files:
          summary: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          decision: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-decision.txt
          todo: artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-todo.txt
"#,
    )
    .expect("write orchestrator yaml");

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
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
memory:
  enabled: true
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");
    bootstrap_memory_runtime_paths(&settings).expect("bootstrap memory runtime");

    let runtime_root = settings
        .resolve_channel_profile_runtime_root("local-default")
        .expect("runtime root");
    let codex_mock = temp.path().join("codex-mock");
    write_script(
        &codex_mock,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"summary\\\":\\\"implemented\\\",\\\"decision\\\":\\\"approve\\\",\\\"todo\\\":\\\"add regression test\\\"}[/workflow_result]\"}}'\n",
    );
    let claude_mock = temp.path().join("claude-mock");
    write_script(&claude_mock, "#!/bin/sh\necho 'ok'\n");
    let binaries = RunnerBinaries {
        anthropic: claude_mock.display().to_string(),
        openai: codex_mock.display().to_string(),
    };

    let inbound = IncomingMessage {
        channel: "local".to_string(),
        channel_profile_id: Some("local-default".to_string()),
        sender: "cli".to_string(),
        sender_id: "cli".to_string(),
        message: "Please ship this".to_string(),
        timestamp: 1,
        message_id: "msg-writeback-1".to_string(),
        conversation_id: Some("chat-1".to_string()),
        is_direct: true,
        is_thread_reply: false,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let _ = process_queued_message_with_runner_binaries(
        temp.path(),
        &settings,
        &inbound,
        1,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(runtime_root.clone()),
            &settings,
        ),
        Some(binaries),
        |_attempt, _request, _orchestrator| None,
    )
    .expect("route and execute");

    let repo =
        direclaw::memory::MemoryRepository::open(&runtime_root.join("memory/memory.db"), "main")
            .expect("open repo");
    let sources = repo.list_sources().expect("list sources");
    assert!(sources
        .iter()
        .any(|source| source.source_type == direclaw::memory::MemorySourceType::ChannelTranscript));
    assert!(sources
        .iter()
        .any(|source| source.source_type == direclaw::memory::MemorySourceType::WorkflowOutput));

    let db = Connection::open(runtime_root.join("memory/memory.db")).expect("open sqlite");
    let embeddings_count: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })
        .expect("count embeddings");
    assert!(
        embeddings_count > 0,
        "expected embedding rows to be persisted"
    );
}

#[test]
fn routing_includes_recent_thread_context_in_selector_request() {
    let temp = tempdir().expect("tempdir");
    let workspace_root = temp.path().join("workspaces");
    let private_workspace = workspace_root.join("main");
    fs::create_dir_all(&private_workspace).expect("create private workspace");
    write_minimal_orchestrator_yaml(&private_workspace);

    let settings = serde_yaml::from_str::<direclaw::config::Settings>(&format!(
        r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {}
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspace_root.display(),
        private_workspace.display()
    ))
    .expect("settings");

    let runtime_root = settings
        .resolve_channel_profile_runtime_root("engineering")
        .expect("runtime root");

    conversation_context::append_inbound_turn(
        &runtime_root,
        "engineering",
        "C1:100.1",
        "m-1",
        10,
        "U42",
        "here is the first message",
    )
    .expect("append inbound");
    conversation_context::append_outbound_turn(
        &runtime_root,
        "engineering",
        "C1:100.1",
        "m-1-out",
        11,
        "orchestrator",
        "Workflow failed.\nrun_id=run-abc",
        Some("run-abc"),
        None,
    )
    .expect("append outbound");

    let inbound = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("engineering".to_string()),
        sender: "dana".to_string(),
        sender_id: "U42".to_string(),
        message: "can you investigate why this failed?".to_string(),
        timestamp: 12,
        message_id: "msg-thread-ctx".to_string(),
        conversation_id: Some("C1:100.1".to_string()),
        is_direct: false,
        is_thread_reply: true,
        is_mentioned: false,
        files: vec![],
        workflow_run_id: None,
        workflow_step_id: None,
    };

    let _ = process_queued_message(
        temp.path(),
        &settings,
        &inbound,
        12,
        &BTreeMap::new(),
        &FunctionRegistry::v1_defaults(
            direclaw::orchestration::run_store::WorkflowRunStore::new(runtime_root.clone()),
            &settings,
        ),
        |_attempt, _request, _orchestrator| {
            Some(
                r#"{
                  "selectorId":"sel-msg-thread-ctx",
                  "status":"selected",
                  "action":"no_response",
                  "selectedWorkflow":null,
                  "diagnosticsScope":null,
                  "functionId":null,
                  "functionArgs":null,
                  "reason":"thread follow-up"
                }"#
                .to_string(),
            )
        },
    )
    .expect("route inbound");

    let request_path =
        runtime_root.join("orchestrator/artifacts/selector-processing-sel-msg-thread-ctx.json");
    let request_json: Value =
        serde_json::from_str(&fs::read_to_string(request_path).expect("read selector request"))
            .expect("parse selector request");
    let thread_context = request_json
        .get("threadContext")
        .and_then(Value::as_str)
        .expect("threadContext should be present");
    assert!(thread_context.contains("first message"));
    assert!(thread_context.contains("run_id=run-abc"));
    assert!(
        thread_context.chars().count() <= ThreadContextLimits::default().max_chars,
        "thread context must be bounded"
    );
}
