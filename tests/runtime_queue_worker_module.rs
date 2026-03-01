use direclaw::config::Settings;
use direclaw::provider::RunnerBinaries;
use direclaw::queue::{IncomingMessage, QueuePaths};
use direclaw::runtime::queue_worker::drain_queue_once_with_binaries;
use direclaw::runtime::state_paths::{bootstrap_state_root, StatePaths};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn runtime_queue_worker_module_exposes_drain_api() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

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

    let binaries = RunnerBinaries {
        anthropic: "unused".to_string(),
        openai: "unused".to_string(),
    };

    let processed = drain_queue_once_with_binaries(&state_root, &settings, 1, &binaries)
        .expect("drain empty queue");
    assert_eq!(processed, 0);
}

#[test]
fn runtime_queue_worker_module_moves_permanent_failures_to_failed_queue() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let old_max = std::env::var_os("DIRECLAW_QUEUE_MAX_REQUEUE_ATTEMPTS");
    std::env::set_var("DIRECLAW_QUEUE_MAX_REQUEUE_ATTEMPTS", "2");

    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let orchestrator_workspace = dir.path().join("orch");
    fs::create_dir_all(&orchestrator_workspace).expect("workspace");
    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: router
default_workflow: triage
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.3-codex-spark
workflows:
  - id: triage
    version: 1
    description: triage workflow
    tags: [triage]
    steps:
      - id: plan
        type: agent_task
        agent: worker
        prompt: plan
        outputs: [summary]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
      - id: finalize
        type: agent_task
        agent: worker
        prompt: finalize
        outputs: [summary]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("orchestrator");

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles:
  eng:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display(),
        orchestrator_workspace = orchestrator_workspace.display()
    ))
    .expect("settings");

    let runtime_root = settings
        .resolve_orchestrator_runtime_root("main")
        .expect("runtime root");
    let queue = QueuePaths::from_state_root(&runtime_root);
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::write(
        queue.incoming.join("msg-1.json"),
        serde_json::to_vec(&IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("eng".to_string()),
            sender: "Dana".to_string(),
            sender_id: "U42".to_string(),
            message: "help".to_string(),
            timestamp: 100,
            message_id: "msg-1".to_string(),
            conversation_id: Some("C123:1700000000.1".to_string()),
            is_direct: true,
            is_thread_reply: true,
            is_mentioned: false,
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        })
        .expect("serialize"),
    )
    .expect("write incoming");

    let claude = dir.path().join("claude-selector");
    fs::write(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-1\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    )
    .expect("claude script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&claude).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&claude, perms).expect("chmod");
    }

    let codex = dir.path().join("codex-fail");
    fs::write(&codex, "#!/bin/sh\necho fail 1>&2\nexit 7\n").expect("codex script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&codex).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&codex, perms).expect("chmod");
    }

    let bins = RunnerBinaries {
        anthropic: claude.display().to_string(),
        openai: codex.display().to_string(),
    };

    let first = drain_queue_once_with_binaries(&state_root, &settings, 1, &bins);
    assert!(first.is_err(), "first attempt should fail");
    let second = drain_queue_once_with_binaries(&state_root, &settings, 1, &bins);
    assert!(
        second.is_err(),
        "second attempt should fail and dead-letter"
    );
    let third = drain_queue_once_with_binaries(&state_root, &settings, 1, &bins)
        .expect("third drain should be clean");
    assert_eq!(third, 0);

    assert!(fs::read_dir(&queue.incoming)
        .expect("incoming")
        .next()
        .is_none());
    assert!(fs::read_dir(&queue.processing)
        .expect("processing")
        .next()
        .is_none());

    let failed_paths: Vec<_> = fs::read_dir(&queue.failed)
        .expect("failed")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert_eq!(failed_paths.len(), 1, "expected one dead-letter item");
    let envelope: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&failed_paths[0]).expect("read")).expect("parse");
    assert_eq!(envelope["failure_attempt"], 2);
    assert_eq!(envelope["message"]["messageId"], "msg-1");

    let outgoing: Vec<direclaw::queue::OutgoingMessage> = fs::read_dir(&queue.outgoing)
        .expect("outgoing")
        .map(|entry| entry.expect("entry").path())
        .map(|path| {
            serde_json::from_str::<direclaw::queue::OutgoingMessage>(
                &fs::read_to_string(path).expect("read outgoing"),
            )
            .expect("parse outgoing")
        })
        .collect();
    assert!(
        outgoing.len() >= 2,
        "expected at least one workflow selection ack and one failure notification"
    );
    assert!(
        outgoing
            .iter()
            .any(|message| message.message_id == "msg-1-workflow-ack"),
        "missing workflow selection ack"
    );
    let failure_notification = outgoing
        .iter()
        .find(|message| message.message_id == "msg-1")
        .expect("missing workflow failure notification");
    assert!(failure_notification.message.contains("Workflow failed"));
    assert!(failure_notification.message.contains("run_id="));
    assert!(failure_notification.message.contains("succeeded steps:"));
    assert!(failure_notification.message.contains("failed steps:"));
    assert!(failure_notification.message.contains("reason:"));

    if let Some(value) = old_max {
        std::env::set_var("DIRECLAW_QUEUE_MAX_REQUEUE_ATTEMPTS", value);
    } else {
        std::env::remove_var("DIRECLAW_QUEUE_MAX_REQUEUE_ATTEMPTS");
    }
}

#[test]
fn runtime_queue_worker_posts_workflow_ack_before_step_execution() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let orchestrator_workspace = dir.path().join("orch");
    fs::create_dir_all(&orchestrator_workspace).expect("workspace");
    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: router
default_workflow: triage
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.3-codex-spark
workflows:
  - id: triage
    version: 1
    description: triage workflow
    tags: [triage]
    steps:
      - id: plan
        type: agent_task
        agent: worker
        prompt: plan
        outputs: [summary]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
      - id: finalize
        type: agent_task
        agent: worker
        prompt: finalize
        outputs: [summary]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("orchestrator");

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles:
  eng:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display(),
        orchestrator_workspace = orchestrator_workspace.display()
    ))
    .expect("settings");

    let runtime_root = settings
        .resolve_orchestrator_runtime_root("main")
        .expect("runtime root");
    let queue = QueuePaths::from_state_root(&runtime_root);
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::write(
        queue.incoming.join("msg-1.json"),
        serde_json::to_vec(&IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("eng".to_string()),
            sender: "Dana".to_string(),
            sender_id: "U42".to_string(),
            message: "help".to_string(),
            timestamp: 100,
            message_id: "msg-1".to_string(),
            conversation_id: Some("C123:1700000000.1".to_string()),
            is_direct: true,
            is_thread_reply: true,
            is_mentioned: false,
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        })
        .expect("serialize"),
    )
    .expect("write incoming");

    let claude = dir.path().join("claude-selector");
    fs::write(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-1\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    )
    .expect("claude script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&claude).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&claude, perms).expect("chmod");
    }

    let codex = dir.path().join("codex-check-ack");
    fs::write(
        &codex,
        format!(
            "#!/bin/sh\nset -eu\nls \"{}/slack_msg-1-workflow-ack_\"*.json >/dev/null 2>&1\necho '{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":\"[workflow_result]{{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\"}}[/workflow_result]\"}}}}'\n",
            queue.outgoing.display()
        ),
    )
    .expect("codex script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&codex).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&codex, perms).expect("chmod");
    }

    let bins = RunnerBinaries {
        anthropic: claude.display().to_string(),
        openai: codex.display().to_string(),
    };

    let processed = drain_queue_once_with_binaries(&state_root, &settings, 1, &bins)
        .expect("workflow should succeed with pre-step ack");
    assert_eq!(processed, 1);

    let outgoing: Vec<direclaw::queue::OutgoingMessage> = fs::read_dir(&queue.outgoing)
        .expect("outgoing list")
        .map(|entry| entry.expect("entry").path())
        .map(|path| {
            serde_json::from_str::<direclaw::queue::OutgoingMessage>(
                &fs::read_to_string(path).expect("read outgoing"),
            )
            .expect("parse outgoing")
        })
        .collect();
    assert!(
        outgoing
            .iter()
            .any(|message| message.message.contains("Actioning workflow triage...")),
        "missing workflow ack message in outgoing queue"
    );
}

#[test]
fn runtime_queue_worker_does_not_post_workflow_ack_for_single_step_workflow() {
    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let orchestrator_workspace = dir.path().join("orch");
    fs::create_dir_all(&orchestrator_workspace).expect("workspace");
    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: router
default_workflow: triage
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.3-codex-spark
workflows:
  - id: triage
    version: 1
    description: triage workflow
    tags: [triage]
    steps:
      - id: plan
        type: agent_task
        agent: worker
        prompt: plan
        outputs: [summary]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("orchestrator");

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles:
  eng:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display(),
        orchestrator_workspace = orchestrator_workspace.display()
    ))
    .expect("settings");

    let runtime_root = settings
        .resolve_orchestrator_runtime_root("main")
        .expect("runtime root");
    let queue = QueuePaths::from_state_root(&runtime_root);
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::write(
        queue.incoming.join("msg-single-step.json"),
        serde_json::to_vec(&IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("eng".to_string()),
            sender: "Dana".to_string(),
            sender_id: "U42".to_string(),
            message: "help".to_string(),
            timestamp: 100,
            message_id: "msg-single-step".to_string(),
            conversation_id: Some("C123:1700000000.10".to_string()),
            is_direct: true,
            is_thread_reply: true,
            is_mentioned: false,
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        })
        .expect("serialize"),
    )
    .expect("write incoming");

    let claude = dir.path().join("claude-selector");
    fs::write(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-single-step\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    )
    .expect("claude script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&claude).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&claude, perms).expect("chmod");
    }

    let codex = dir.path().join("codex-success");
    fs::write(
        &codex,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    )
    .expect("codex script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&codex).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&codex, perms).expect("chmod");
    }

    let bins = RunnerBinaries {
        anthropic: claude.display().to_string(),
        openai: codex.display().to_string(),
    };
    let processed = drain_queue_once_with_binaries(&state_root, &settings, 1, &bins)
        .expect("workflow should succeed");
    assert_eq!(processed, 1);

    let outgoing: Vec<direclaw::queue::OutgoingMessage> = fs::read_dir(&queue.outgoing)
        .expect("outgoing list")
        .map(|entry| entry.expect("entry").path())
        .map(|path| {
            serde_json::from_str::<direclaw::queue::OutgoingMessage>(
                &fs::read_to_string(path).expect("read outgoing"),
            )
            .expect("parse outgoing")
        })
        .collect();
    assert!(
        outgoing
            .iter()
            .all(|message| !message.message.contains("Actioning workflow triage...")),
        "single-step workflow should not emit pre-step ack"
    );
}

#[test]
fn runtime_queue_worker_does_not_post_failure_notification_on_requeue_attempt() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let old_max = std::env::var_os("DIRECLAW_QUEUE_MAX_REQUEUE_ATTEMPTS");
    std::env::set_var("DIRECLAW_QUEUE_MAX_REQUEUE_ATTEMPTS", "3");

    let dir = tempdir().expect("tempdir");
    let state_root = dir.path().join(".direclaw");
    bootstrap_state_root(&StatePaths::new(&state_root)).expect("bootstrap");

    let orchestrator_workspace = dir.path().join("orch");
    fs::create_dir_all(&orchestrator_workspace).expect("workspace");
    fs::write(
        orchestrator_workspace.join("orchestrator.yaml"),
        r#"
id: main
selector_agent: router
default_workflow: triage
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.3-codex-spark
workflows:
  - id: triage
    version: 1
    description: triage workflow
    tags: [triage]
    steps:
      - id: plan
        type: agent_task
        agent: worker
        prompt: plan
        outputs: [summary]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
      - id: finalize
        type: agent_task
        agent: worker
        prompt: finalize
        outputs: [summary]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("orchestrator");

    let settings: Settings = serde_yaml::from_str(&format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  main:
    private_workspace: {orchestrator_workspace}
    shared_access: []
channel_profiles:
  eng:
    channel: slack
    orchestrator_id: main
    slack_app_user_id: UAPP
    require_mention_in_channels: true
monitoring: {{}}
channels: {{}}
"#,
        workspace = dir.path().display(),
        orchestrator_workspace = orchestrator_workspace.display()
    ))
    .expect("settings");

    let runtime_root = settings
        .resolve_orchestrator_runtime_root("main")
        .expect("runtime root");
    let queue = QueuePaths::from_state_root(&runtime_root);
    fs::create_dir_all(&queue.incoming).expect("incoming");
    fs::write(
        queue.incoming.join("msg-2.json"),
        serde_json::to_vec(&IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("eng".to_string()),
            sender: "Dana".to_string(),
            sender_id: "U42".to_string(),
            message: "help".to_string(),
            timestamp: 100,
            message_id: "msg-2".to_string(),
            conversation_id: Some("C123:1700000000.2".to_string()),
            is_direct: true,
            is_thread_reply: true,
            is_mentioned: false,
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        })
        .expect("serialize"),
    )
    .expect("write incoming");

    let claude = dir.path().join("claude-selector");
    fs::write(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-msg-2\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    )
    .expect("claude script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&claude).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&claude, perms).expect("chmod");
    }

    let codex = dir.path().join("codex-fail");
    fs::write(&codex, "#!/bin/sh\necho fail 1>&2\nexit 7\n").expect("codex script");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&codex).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&codex, perms).expect("chmod");
    }

    let bins = RunnerBinaries {
        anthropic: claude.display().to_string(),
        openai: codex.display().to_string(),
    };
    let first = drain_queue_once_with_binaries(&state_root, &settings, 1, &bins);
    assert!(first.is_err(), "first attempt should fail and requeue");

    assert!(
        fs::read_dir(&queue.failed)
            .expect("failed")
            .next()
            .is_none(),
        "should not dead-letter on first attempt"
    );
    assert!(
        fs::read_dir(&queue.incoming)
            .expect("incoming")
            .next()
            .is_some(),
        "should requeue for retry"
    );

    let outgoing: Vec<direclaw::queue::OutgoingMessage> = fs::read_dir(&queue.outgoing)
        .expect("outgoing")
        .map(|entry| entry.expect("entry").path())
        .map(|path| {
            serde_json::from_str::<direclaw::queue::OutgoingMessage>(
                &fs::read_to_string(path).expect("read outgoing"),
            )
            .expect("parse outgoing")
        })
        .filter(|message| message.message_id.starts_with("msg-2"))
        .collect();
    assert_eq!(outgoing.len(), 1, "only selection ack should be emitted");
    assert!(
        outgoing[0].message.contains("Actioning workflow triage..."),
        "expected workflow selection ack"
    );

    if let Some(value) = old_max {
        std::env::set_var("DIRECLAW_QUEUE_MAX_REQUEUE_ATTEMPTS", value);
    } else {
        std::env::remove_var("DIRECLAW_QUEUE_MAX_REQUEUE_ATTEMPTS");
    }
}
