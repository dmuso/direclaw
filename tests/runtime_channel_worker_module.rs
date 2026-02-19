use direclaw::runtime::channel_worker::{tick_slack_worker, PollingDefaults};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};
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

#[test]
fn runtime_channel_worker_handles_slack_rate_limit_without_error() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock slack");
    let addr = listener.local_addr().expect("mock addr");
    let server = thread::spawn(move || {
        for expected in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .expect("read request line");

            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("read header");
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }

            let response = if expected == 0 {
                r#"HTTP/1.1 200 OK
Content-Type: application/json
Content-Length: 11
Connection: close

{"ok":true}"#
            } else {
                r#"HTTP/1.1 429 Too Many Requests
Content-Type: application/json
Retry-After: 1
Content-Length: 34
Connection: close

{"ok":false,"error":"ratelimited"}"#
            };
            let wire = response.replace('\n', "\r\n");
            stream
                .write_all(wire.as_bytes())
                .expect("write mock response");
        }
    });

    std::env::set_var("DIRECLAW_SLACK_API_BASE", format!("http://{addr}/api"));
    std::env::set_var("SLACK_BOT_TOKEN", "xoxb-test");
    std::env::set_var("SLACK_APP_TOKEN", "xapp-test");

    let dir = tempdir().expect("tempdir");
    let settings = serde_yaml::from_str(
        r#"
workspaces_path: /tmp
shared_workspaces: {}
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
monitoring: {}
channels:
  slack:
    enabled: true
"#,
    )
    .expect("parse settings");

    let started = Instant::now();
    let result = tick_slack_worker(dir.path(), &settings);
    let elapsed = started.elapsed();

    std::env::remove_var("DIRECLAW_SLACK_API_BASE");
    std::env::remove_var("SLACK_BOT_TOKEN");
    std::env::remove_var("SLACK_APP_TOKEN");
    server.join().expect("join mock");

    assert!(
        result.is_ok(),
        "rate-limit tick should not fail worker state: {result:?}"
    );
    assert!(
        elapsed >= Duration::from_secs(1),
        "expected Retry-After cooldown to be respected, elapsed={elapsed:?}"
    );
}
