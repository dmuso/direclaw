use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tempfile::tempdir;

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
    let workspace = home.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        home.join(".direclaw.yaml"),
        format!(
            r#"
workspace_path: {workspace}
shared_workspaces: {{}}
orchestrators: {{}}
channel_profiles: {{}}
monitoring:
  heartbeat_interval: 1
channels: {{}}
auth_sync:
  enabled: false
"#,
            workspace = workspace.display()
        ),
    )
    .expect("settings");
}

fn wait_for_status_line(home: &Path, needle: &str, timeout: Duration) {
    let start = Instant::now();
    loop {
        let output = run(home, &["status"], &[]);
        assert_ok(&output);
        if stdout(&output).contains(needle) {
            return;
        }
        assert!(
            start.elapsed() < timeout,
            "timed out waiting for `{needle}`; last status:\n{}",
            stdout(&output)
        );
        std::thread::sleep(Duration::from_millis(100));
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
    wait_for_status_line(home, "running=true", Duration::from_secs(3));

    let duplicate = run(home, &["start"], &[]);
    assert_err_contains(&duplicate, "already running");

    wait_for_status_line(
        home,
        "worker:queue_processor.state=running",
        Duration::from_secs(3),
    );
    wait_for_status_line(
        home,
        "worker:orchestrator_dispatcher.state=running",
        Duration::from_secs(3),
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
fn restart_performs_full_stop_start_and_refreshes_runtime_start() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings(home);

    let started = run(home, &["start"], &[]);
    assert_ok(&started);
    wait_for_status_line(home, "running=true", Duration::from_secs(3));

    let status_before = run(home, &["status"], &[]);
    assert_ok(&status_before);
    let before_text = stdout(&status_before);
    let before_started_at = before_text
        .lines()
        .find_map(|line| line.strip_prefix("started_at="))
        .expect("started_at line")
        .to_string();

    std::thread::sleep(Duration::from_secs(1));

    let restarted = run(home, &["restart"], &[]);
    assert_ok(&restarted);
    wait_for_status_line(home, "running=true", Duration::from_secs(3));
    wait_for_status_line(
        home,
        "worker:queue_processor.state=running",
        Duration::from_secs(3),
    );

    let status_after = run(home, &["status"], &[]);
    assert_ok(&status_after);
    let after_text = stdout(&status_after);
    let after_started_at = after_text
        .lines()
        .find_map(|line| line.strip_prefix("started_at="))
        .expect("started_at line")
        .to_string();

    assert_ne!(before_started_at, after_started_at);
    assert!(after_text.contains("worker:queue_processor.state=running"));

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

    wait_for_status_line(
        home,
        "worker:queue_processor.state=error",
        Duration::from_secs(4),
    );
    wait_for_status_line(
        home,
        "worker:queue_processor.last_error=fault injection requested",
        Duration::from_secs(2),
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

    for _ in 0..12 {
        let started = run(home, &["start"], &[]);
        assert_ok(&started);
        wait_for_status_line(home, "running=true", Duration::from_secs(3));

        for _ in 0..4 {
            let status = run(home, &["status"], &[]);
            assert_ok(&status);
        }

        let restarted = run(home, &["restart"], &[]);
        assert_ok(&restarted);
        wait_for_status_line(home, "running=true", Duration::from_secs(3));

        let stopped = run(home, &["stop"], &[]);
        assert_ok(&stopped);
        wait_for_status_line(home, "running=false", Duration::from_secs(3));
    }
}

#[test]
fn slow_shutdown_fault_injection_reports_timeout_state_and_log() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();
    write_settings(home);

    let started = run(
        home,
        &["start"],
        &[("DIRECLAW_SLOW_SHUTDOWN_WORKER", "queue_processor")],
    );
    assert_ok(&started);
    wait_for_status_line(home, "running=true", Duration::from_secs(3));
    let stop_file = home.join(".direclaw/daemon/stop");
    fs::write(&stop_file, b"stop").expect("write stop signal");
    wait_for_status_line(home, "running=false", Duration::from_secs(10));

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
    write_settings(home);

    let started = run(home, &["start"], &[]);
    assert_ok(&started);
    wait_for_status_line(home, "running=true", Duration::from_secs(3));
    wait_for_status_line(
        home,
        "worker:queue_processor.state=running",
        Duration::from_secs(3),
    );
    wait_for_status_line(
        home,
        "worker:orchestrator_dispatcher.state=running",
        Duration::from_secs(3),
    );

    let status = run(home, &["status"], &[]);
    assert_ok(&status);
    let status_text = stdout(&status);
    assert!(status_text.contains("ownership=running"));
    assert!(status_text.contains("worker:queue_processor.state=running"));
    assert!(status_text.contains("worker:orchestrator_dispatcher.state=running"));

    let logs = run(home, &["logs"], &[]);
    assert_ok(&logs);
    assert!(stdout(&logs).contains("runtime.log"));

    let runtime_log = read_runtime_log(home);
    assert!(runtime_log.contains("\"event\":\"supervisor.started\""));
    assert!(runtime_log.contains("\"event\":\"worker.started\""));

    stop_if_running(home);
}
