use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::thread;
use tempfile::tempdir;

fn run_with_env(home: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_direclaw"));
    cmd.args(args).env("HOME", home);
    for (key, value) in envs {
        cmd.env(key, value);
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
    let text = format!("{}{}", stdout(output), stderr(output));
    assert!(
        text.contains(needle),
        "expected error to contain `{needle}`, got:\n{text}"
    );
}

fn spawn_release_server(status_line: &str, response_body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let status_line = status_line.to_string();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .expect("read request line");
        assert!(
            request_line.contains("/repos/acme/direclaw/releases/latest"),
            "unexpected request line: {request_line}"
        );

        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read header line");
            if line == "\r\n" || line.is_empty() {
                break;
            }
        }

        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    format!("http://{}", addr)
}

#[test]
fn update_check_reports_release_metadata_and_update_available() {
    let temp = tempdir().expect("tempdir");
    let api_url = spawn_release_server(
        "200 OK",
        r#"{"tag_name":"v999.1.0","html_url":"https://example.com/release","published_at":"2026-02-14T01:02:03Z","prerelease":false,"draft":false,"assets":[{"name":"direclaw-v999.1.0-x86_64-unknown-linux-gnu.tar.gz"},{"name":"checksums.txt"}]}"#
            .to_string(),
    );

    let output = run_with_env(
        temp.path(),
        &["update", "check"],
        &[
            ("DIRECLAW_UPDATE_API_URL", &api_url),
            ("DIRECLAW_UPDATE_REPO", "acme/direclaw"),
        ],
    );
    assert_ok(&output);

    let out = stdout(&output);
    assert!(out.contains("update_check=ok"), "unexpected output:\n{out}");
    assert!(
        out.contains("repository=acme/direclaw"),
        "unexpected output:\n{out}"
    );
    assert!(
        out.contains("latest_version=999.1.0"),
        "unexpected output:\n{out}"
    );
    assert!(
        out.contains("update_available=true"),
        "unexpected output:\n{out}"
    );
    assert!(
        out.contains("release_url=https://example.com/release"),
        "unexpected output:\n{out}"
    );
    assert!(
        out.contains("assets=checksums.txt,direclaw-v999.1.0-x86_64-unknown-linux-gnu.tar.gz"),
        "unexpected output:\n{out}"
    );
}

#[test]
fn update_check_reports_up_to_date_for_matching_version() {
    let temp = tempdir().expect("tempdir");
    let current = env!("CARGO_PKG_VERSION");
    let body = format!(
        "{{\"tag_name\":\"v{}\",\"html_url\":\"https://example.com/release\",\"published_at\":\"2026-02-14T01:02:03Z\",\"prerelease\":false,\"draft\":false,\"assets\":[]}}",
        current
    );
    let api_url = spawn_release_server("200 OK", body);

    let output = run_with_env(
        temp.path(),
        &["update", "check"],
        &[
            ("DIRECLAW_UPDATE_API_URL", &api_url),
            ("DIRECLAW_UPDATE_REPO", "acme/direclaw"),
        ],
    );
    assert_ok(&output);

    let out = stdout(&output);
    assert!(
        out.contains("update_available=false"),
        "unexpected output:\n{out}"
    );
    assert!(!out.contains("remediation="), "unexpected output:\n{out}");
}

#[test]
fn update_check_fails_on_invalid_release_metadata() {
    let temp = tempdir().expect("tempdir");
    let api_url = spawn_release_server("200 OK", "{invalid-json".to_string());

    let output = run_with_env(
        temp.path(),
        &["update", "check"],
        &[
            ("DIRECLAW_UPDATE_API_URL", &api_url),
            ("DIRECLAW_UPDATE_REPO", "acme/direclaw"),
        ],
    );

    assert_err_contains(&output, "update check failed to parse release metadata");
    assert_err_contains(
        &output,
        "remediation: verify network access and set DIRECLAW_UPDATE_REPO/DIRECLAW_UPDATE_API_URL if needed",
    );
}

#[test]
fn update_check_rejects_draft_release_metadata() {
    let temp = tempdir().expect("tempdir");
    let api_url = spawn_release_server(
        "200 OK",
        r#"{"tag_name":"v999.1.0","html_url":"https://example.com/release","published_at":"2026-02-14T01:02:03Z","prerelease":false,"draft":true,"assets":[]}"#
            .to_string(),
    );

    let output = run_with_env(
        temp.path(),
        &["update", "check"],
        &[
            ("DIRECLAW_UPDATE_API_URL", &api_url),
            ("DIRECLAW_UPDATE_REPO", "acme/direclaw"),
        ],
    );

    assert_err_contains(
        &output,
        "update check failed: latest release is a draft and cannot be used for updates",
    );
}

#[test]
fn update_check_fails_on_non_200_release_metadata_response() {
    let temp = tempdir().expect("tempdir");
    let api_url = spawn_release_server("404 Not Found", "{}".to_string());

    let output = run_with_env(
        temp.path(),
        &["update", "check"],
        &[
            ("DIRECLAW_UPDATE_API_URL", &api_url),
            ("DIRECLAW_UPDATE_REPO", "acme/direclaw"),
        ],
    );

    assert_err_contains(&output, "status code 404");
    assert_err_contains(
        &output,
        "remediation: verify network access and set DIRECLAW_UPDATE_REPO/DIRECLAW_UPDATE_API_URL if needed",
    );
}
