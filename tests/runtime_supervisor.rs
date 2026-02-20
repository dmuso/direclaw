use direclaw::queue::IncomingMessage;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn run(home: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_direclaw"));
    cmd.args(args).env("HOME", home);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.output().expect("run direclaw")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn assert_err_contains(output: &Output, needle: &str) {
    assert!(
        !output.status.success(),
        "expected failure, stdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
    let combined = format!("{}{}", stdout(output), stderr(output));
    assert!(
        combined.contains(needle),
        "expected `{needle}` in output:\n{combined}"
    );
}

fn write_settings(home: &Path) {
    write_settings_with_heartbeat_interval(home, 0);
}

fn write_settings_with_heartbeat_interval(home: &Path, heartbeat_interval: u64) {
    let workspace = home.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("config dir");
    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators: {{}}
channel_profiles: {{}}
monitoring:
  heartbeat_interval: {heartbeat_interval}
channels: {{}}
auth_sync:
  enabled: false
"#,
            workspace = workspace.display(),
            heartbeat_interval = heartbeat_interval
        ),
    )
    .expect("settings");
}

fn write_workflow_settings(home: &Path) {
    let workspace = home.join("workspace");
    let orch_workspace = home.join("orch");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&orch_workspace).expect("orchestrator workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("config dir");
    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  eng_orchestrator:
    private_workspace: {orch_workspace}
    shared_access: []
channel_profiles:
  eng:
    channel: slack
    orchestrator_id: eng_orchestrator
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring:
  heartbeat_interval: 1
channels:
  slack:
    enabled: false
auth_sync:
  enabled: false
"#,
            workspace = workspace.display(),
            orch_workspace = orch_workspace.display(),
        ),
    )
    .expect("settings");
    fs::write(
        orch_workspace.join("orchestrator.yaml"),
        r#"
id: eng_orchestrator
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
    model: gpt-5.2
workflows:
  - id: triage
    version: 1
    description: triage workflow for supervisor recovery tests
    tags: [triage, recovery]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
        outputs: [summary, artifact]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("orchestrator");
}

fn write_invalid_workflow_settings(home: &Path) {
    let workspace = home.join("workspace");
    let orch_workspace = home.join("orch");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&orch_workspace).expect("orchestrator workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("config dir");
    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  eng_orchestrator:
    private_workspace: {orch_workspace}
    shared_access: []
channel_profiles:
  eng:
    channel: slack
    orchestrator_id: eng_orchestrator
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring:
  heartbeat_interval: 1
channels:
  slack:
    enabled: false
auth_sync:
  enabled: false
"#,
            workspace = workspace.display(),
            orch_workspace = orch_workspace.display(),
        ),
    )
    .expect("settings");
    fs::write(
        orch_workspace.join("orchestrator.yaml"),
        r#"
id: eng_orchestrator
selector_agent: router
default_workflow: default
selection_max_retries: 1
selector_timeout_seconds: 30
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  worker:
    provider: openai
    model: gpt-5.2
workflows:
  - id: default
    version: 1
    description: ""
    tags: [triage]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
        outputs: [summary, artifact]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("orchestrator");
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }
}

fn write_slack_settings(home: &Path, slack_enabled: bool) {
    let workspace = home.join("workspace");
    let orchestrator_workspace = workspace.join("main");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&orchestrator_workspace).expect("orchestrator workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("config dir");
    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspaces_path: {workspace}
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
monitoring:
  heartbeat_interval: 1
channels:
  slack:
    enabled: {slack_enabled}
auth_sync:
  enabled: false
"#,
            workspace = workspace.display(),
            slack_enabled = slack_enabled
        ),
    )
    .expect("settings");
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
    model: gpt-5.2
workflows:
  - id: triage
    version: 1
    description: triage workflow for slack status tests
    tags: [triage, slack]
    steps:
      - id: start
        type: agent_task
        agent: worker
        prompt: start
        outputs: [summary, artifact]
        output_files:
          summary: outputs/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt
          artifact: outputs/{{workflow.step_id}}-{{workflow.attempt}}.txt
"#,
    )
    .expect("orchestrator");
}

struct MockSlackServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockSlackServer {
    fn start<F>(expected_requests: usize, responder: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_thread = Arc::clone(&requests);
        let responder = Arc::new(responder);

        let handle = thread::spawn(move || {
            for _ in 0..expected_requests {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut request_line = String::new();
                reader
                    .read_line(&mut request_line)
                    .expect("read request line");
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();

                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("read header");
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if lower.starts_with("content-length:") {
                        content_length = line
                            .split_once(':')
                            .map(|(_, v)| v.trim().parse::<usize>().unwrap_or(0))
                            .unwrap_or(0);
                    }
                }

                let mut body_buf = vec![0_u8; content_length];
                if content_length > 0 {
                    reader.read_exact(&mut body_buf).expect("read body");
                }

                requests_for_thread
                    .lock()
                    .expect("lock requests")
                    .push(path.clone());
                let response_body = responder(&path);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        Self {
            base_url: format!("http://{}", addr),
            requests,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> Vec<String> {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join mock server");
        }
        self.requests.lock().expect("lock requests").clone()
    }
}

fn runtime_state_json(home: &Path) -> Option<serde_json::Value> {
    let path = home.join(".direclaw/daemon/runtime.json");
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn wait_for_runtime_state(
    home: &Path,
    timeout: Duration,
    predicate: impl Fn(&serde_json::Value) -> bool,
    message: &str,
) {
    let start = Instant::now();
    loop {
        if runtime_state_json(home).as_ref().is_some_and(&predicate) {
            return;
        }
        assert!(
            start.elapsed() < timeout,
            "timed out waiting for runtime state: {message}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn stop_if_running(home: &Path) {
    let _ = run(home, &["stop"], &[]);
}

fn read_runtime_log(home: &Path) -> String {
    let path = home.join(".direclaw/logs/runtime.log");
    fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn start_stop_idempotency_and_duplicate_start_protection() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings(home);

    let started = run(home, &["start"], &[]);
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );

    let duplicate = run(home, &["start"], &[]);
    assert_err_contains(&duplicate, "already running");

    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["workers"]["queue_processor"]["state"] == "running",
        "worker queue_processor running",
    );
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["workers"]["orchestrator_dispatcher"]["state"] == "running",
        "worker orchestrator_dispatcher running",
    );
    let status = run(home, &["status"], &[]);
    assert_ok(&status);
    let text = stdout(&status);
    assert!(text.contains("worker:queue_processor.state=running"));
    assert!(text.contains("worker:orchestrator_dispatcher.state=running"));

    let stopped = run(home, &["stop"], &[]);
    assert_ok(&stopped);
    let stopped_again = run(home, &["stop"], &[]);
    assert_ok(&stopped_again);
    assert!(stdout(&stopped_again).contains("running=false"));
}

#[test]
fn start_fails_fast_when_orchestrator_config_is_invalid() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_invalid_workflow_settings(home);

    let started = run(home, &["start"], &[]);
    assert_err_contains(
        &started,
        "orchestrator validation failed: workflow `default` requires non-empty `description`",
    );
}

#[test]
fn restart_performs_full_stop_start_and_refreshes_runtime_start() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings(home);

    let started = run(home, &["start"], &[]);
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );

    let status_before = run(home, &["status"], &[]);
    assert_ok(&status_before);
    let before_text = stdout(&status_before);
    let before_started_at = before_text
        .lines()
        .find_map(|line| line.strip_prefix("started_at="))
        .expect("started_at line")
        .to_string();
    let before_pid = before_text
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .expect("pid line")
        .to_string();

    let restarted = run(home, &["restart"], &[]);
    assert_ok(&restarted);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["workers"]["queue_processor"]["state"] == "running",
        "worker queue_processor running",
    );

    let status_after = run(home, &["status"], &[]);
    assert_ok(&status_after);
    let after_text = stdout(&status_after);
    let after_started_at = after_text
        .lines()
        .find_map(|line| line.strip_prefix("started_at="))
        .expect("started_at line")
        .to_string();
    let after_pid = after_text
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .expect("pid line")
        .to_string();

    assert!(before_started_at != after_started_at || before_pid != after_pid);
    assert!(after_text.contains("worker:queue_processor.state=running"));

    stop_if_running(home);
}

#[test]
fn start_recovers_processing_entry_and_processes_recovered_message() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_workflow_settings(home);

    let bin_dir = home.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    let claude = bin_dir.join("claude");
    let codex = bin_dir.join("codex");
    write_script(
        &claude,
        "#!/bin/sh\necho '{\"selectorId\":\"sel-recovered\",\"status\":\"selected\",\"action\":\"workflow_start\",\"selectedWorkflow\":\"triage\"}'\n",
    );
    write_script(
        &codex,
        "#!/bin/sh\necho '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[workflow_result]{\\\"status\\\":\\\"complete\\\",\\\"summary\\\":\\\"ok\\\",\\\"artifact\\\":\\\"ok\\\"}[/workflow_result]\"}}'\n",
    );

    let runtime_root = home.join("orch");
    let processing_dir = runtime_root.join("queue/processing");
    fs::create_dir_all(runtime_root.join("queue/incoming")).expect("incoming dir");
    fs::create_dir_all(&processing_dir).expect("processing dir");
    let stale = IncomingMessage {
        channel: "slack".to_string(),
        channel_profile_id: Some("eng".to_string()),
        sender: "Dana".to_string(),
        sender_id: "U42".to_string(),
        message: "recover me".to_string(),
        timestamp: 100,
        message_id: "msg-recovered-supervisor".to_string(),
        conversation_id: Some("thread-recovered-supervisor".to_string()),
        is_direct: false,
        is_thread_reply: false,
        is_mentioned: false,
        files: Vec::new(),
        workflow_run_id: None,
        workflow_step_id: None,
    };
    fs::write(
        processing_dir.join("stale-msg.json"),
        serde_json::to_vec(&stale).expect("serialize stale"),
    )
    .expect("write stale queue entry");

    let started = run(
        home,
        &["start"],
        &[
            (
                "DIRECLAW_PROVIDER_BIN_ANTHROPIC",
                claude.to_str().expect("claude path"),
            ),
            (
                "DIRECLAW_PROVIDER_BIN_OPENAI",
                codex.to_str().expect("codex path"),
            ),
        ],
    );
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(4),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );

    let outgoing_dir = runtime_root.join("queue/outgoing");
    fs::create_dir_all(&outgoing_dir).expect("outgoing dir");
    let start = Instant::now();
    while fs::read_dir(&outgoing_dir)
        .expect("outgoing")
        .next()
        .is_none()
    {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "recovered message was not processed"
        );
        thread::sleep(Duration::from_millis(20));
    }

    let runtime_log = read_runtime_log(home);
    assert!(runtime_log.contains("\"event\":\"queue.recovered\""));
    stop_if_running(home);
}

#[test]
fn worker_failure_reports_degraded_health_and_logs() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings(home);

    let started = run(
        home,
        &["start"],
        &[("DIRECLAW_FAIL_WORKER", "queue_processor")],
    );
    assert_ok(&started);

    wait_for_runtime_state(
        home,
        Duration::from_secs(4),
        |state| state["workers"]["queue_processor"]["state"] == "error",
        "worker queue_processor error",
    );
    wait_for_runtime_state(
        home,
        Duration::from_secs(2),
        |state| state["workers"]["queue_processor"]["last_error"] == "fault injection requested",
        "worker queue_processor last_error=fault injection requested",
    );

    let logs = run(home, &["logs"], &[]);
    assert_ok(&logs);
    let logs_text = stdout(&logs);
    assert!(logs_text.contains("runtime.log"));

    stop_if_running(home);
}

#[test]
fn repeated_start_status_restart_never_corrupts_runtime_state() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings(home);

    let started = run(home, &["start"], &[]);
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );

    let status = run(home, &["status"], &[]);
    assert_ok(&status);

    let restarted = run(home, &["restart"], &[]);
    assert_ok(&restarted);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );

    let stopped = run(home, &["stop"], &[]);
    assert_ok(&stopped);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(false),
        "running=false",
    );
}

#[test]
fn slow_shutdown_fault_injection_reports_timeout_state_and_log() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings(home);

    let started = run(
        home,
        &["start"],
        &[
            ("DIRECLAW_SLOW_SHUTDOWN_WORKER", "queue_processor"),
            ("DIRECLAW_SHUTDOWN_TIMEOUT_MILLISECONDS", "100"),
            ("DIRECLAW_SLOW_SHUTDOWN_DELAY_MILLISECONDS", "200"),
        ],
    );
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );
    let stop_file = home.join(".direclaw/daemon/stop");
    fs::write(&stop_file, b"stop").expect("write stop signal");
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(false),
        "running=false",
    );

    let status = run(home, &["status"], &[]);
    assert_ok(&status);
    let text = stdout(&status);
    assert!(text.contains("last_error=shutdown timeout waiting for workers:"));
    assert!(text.contains("worker:queue_processor.last_error=shutdown timeout"));

    let runtime_log = read_runtime_log(home);
    assert!(runtime_log.contains("\"event\":\"supervisor.shutdown.timeout\""));
}

#[test]
fn status_and_logs_expose_stable_operational_fields() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings_with_heartbeat_interval(home, 1);

    let started = run(home, &["start"], &[]);
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["workers"]["queue_processor"]["state"] == "running",
        "worker queue_processor running",
    );
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["workers"]["orchestrator_dispatcher"]["state"] == "running",
        "worker orchestrator_dispatcher running",
    );
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["workers"]["heartbeat"]["state"] == "running",
        "worker heartbeat running",
    );

    let status = run(home, &["status"], &[]);
    assert_ok(&status);
    let status_text = stdout(&status);
    assert!(status_text.contains("ownership=running"));
    assert!(status_text.contains("worker:queue_processor.state=running"));
    assert!(status_text.contains("worker:orchestrator_dispatcher.state=running"));
    assert!(status_text.contains("worker:heartbeat.state=running"));
    assert!(!status_text.contains("worker:heartbeat.last_heartbeat=none"));

    let logs = run(home, &["logs"], &[]);
    assert_ok(&logs);
    assert!(stdout(&logs).contains("runtime.log"));

    let runtime_log = read_runtime_log(home);
    assert!(runtime_log.contains("\"event\":\"supervisor.started\""));
    assert!(runtime_log.contains("\"event\":\"worker.started\""));

    stop_if_running(home);
}

#[test]
fn heartbeat_worker_respects_disabled_interval() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings_with_heartbeat_interval(home, 0);

    let started = run(home, &["start"], &[]);
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );

    let status = run(home, &["status"], &[]);
    assert_ok(&status);
    let status_text = stdout(&status);
    assert!(
        !status_text.contains("worker:heartbeat.state="),
        "heartbeat worker should not start when disabled:\n{status_text}"
    );

    stop_if_running(home);
}

#[test]
fn heartbeat_tick_failure_is_non_fatal_to_supervisor() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings_with_heartbeat_interval(home, 1);

    let started = run(home, &["start"], &[("DIRECLAW_FAIL_HEARTBEAT_TICK", "1")]);
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["running"] == serde_json::Value::Bool(true),
        "running=true",
    );
    wait_for_runtime_state(
        home,
        Duration::from_secs(4),
        |state| state["workers"]["heartbeat"]["state"] == "error",
        "worker heartbeat error",
    );
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["workers"]["queue_processor"]["state"] == "running",
        "worker queue_processor running",
    );

    let status = run(home, &["status"], &[]);
    assert_ok(&status);
    let status_text = stdout(&status);
    assert!(status_text.contains("running=true"));
    assert!(status_text
        .contains("worker:heartbeat.last_error=fault injection requested for heartbeat tick"));
    assert!(status_text.contains("worker:queue_processor.state=running"));

    stop_if_running(home);
}

#[test]
fn status_reports_profile_health_when_slack_disabled() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_slack_settings(home, false);

    let status = run(home, &["status"], &[]);
    assert_ok(&status);
    let text = stdout(&status);
    assert!(text.contains("slack_profile:slack_main.health=disabled"));
    assert!(text.contains("slack_profile:slack_main.reason=slack channel disabled in settings"));
}

#[test]
fn slack_worker_start_reports_profile_scoped_missing_credentials() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_slack_settings(home, true);

    let started = run(home, &["start"], &[]);
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["workers"]["channel:slack"]["state"] == "error",
        "worker channel:slack error",
    );

    let status = run(home, &["status"], &[]);
    assert_ok(&status);
    let text = stdout(&status);
    assert!(text.contains("slack_profile:slack_main.health=auth_missing"));
    assert!(text.contains(
        "slack_profile:slack_main.reason=missing required env var `SLACK_BOT_TOKEN_SLACK_MAIN` for slack profile `slack_main`"
    ));

    stop_if_running(home);
}

#[test]
fn slack_worker_running_is_exposed_in_status() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_slack_settings(home, true);

    let server = MockSlackServer::start(3, |path| {
        if path.starts_with("/api/auth.test") {
            return r#"{"ok":true}"#.to_string();
        }
        if path.starts_with("/api/apps.connections.open") {
            return r#"{"ok":true,"url":"wss://example"}"#.to_string();
        }
        if path.starts_with("/api/conversations.list") {
            return r#"{"ok":true,"conversations":[],"response_metadata":{"next_cursor":""}}"#
                .to_string();
        }
        r#"{"ok":false,"error":"unexpected_path"}"#.to_string()
    });
    let slack_api_base = format!("{}/api", server.base_url);
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");
    std::env::set_var("DIRECLAW_SLACK_API_BASE", &slack_api_base);

    let started = run(home, &["start"], &[]);
    assert_ok(&started);
    wait_for_runtime_state(
        home,
        Duration::from_secs(3),
        |state| state["workers"]["channel:slack"]["state"] == "running",
        "worker channel:slack running",
    );

    let running_status = run(home, &["status"], &[]);
    assert_ok(&running_status);
    let running_text = stdout(&running_status);
    assert!(running_text.contains("slack_profile:slack_main.health=running"));

    assert!(!server.finish().is_empty());
    stop_if_running(home);
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("DIRECLAW_SLACK_API_BASE");
}

#[test]
fn slack_worker_api_failure_is_exposed_in_status() {
    let _env_guard = ENV_LOCK.lock().expect("env lock");
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_slack_settings(home, true);

    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");
    std::env::set_var("DIRECLAW_SLACK_API_BASE", "http://127.0.0.1:9/api");
    let started_api_fail = run(home, &["start"], &[]);
    assert_ok(&started_api_fail);
    wait_for_runtime_state(
        home,
        Duration::from_secs(4),
        |state| state["workers"]["channel:slack"]["state"] == "error",
        "worker channel:slack error",
    );

    let api_status = run(home, &["status"], &[]);
    assert_ok(&api_status);
    let api_text = stdout(&api_status);
    assert!(api_text.contains("slack_profile:slack_main.health=api_failure"));

    stop_if_running(home);
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    std::env::remove_var("DIRECLAW_SLACK_API_BASE");
}
