use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;

fn run(home: &Path, args: &[&str], path_prefix: Option<&Path>, op_token: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_direclaw"));
    cmd.args(args).env("HOME", home);
    if let Some(prefix) = path_prefix {
        let base_path = std::env::var("PATH").unwrap_or_default();
        let merged = format!("{}:{}", prefix.display(), base_path);
        cmd.env("PATH", merged);
    }
    match op_token {
        Some(value) => {
            cmd.env("OP_SERVICE_ACCOUNT_TOKEN", value);
        }
        None => {
            cmd.env_remove("OP_SERVICE_ACCOUNT_TOKEN");
        }
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

fn write_op_mock(bin_path: &Path) {
    fs::write(
        bin_path,
        r#"#!/bin/sh
if [ -z "${OP_SERVICE_ACCOUNT_TOKEN}" ]; then
  echo "missing token" 1>&2
  exit 3
fi
if [ "$1" = "read" ] && [ "$2" = "op://direclaw/codex-auth" ]; then
  printf "%s" "{\"token\":\"abc123\"}"
  exit 0
fi
echo "bad args: $*" 1>&2
exit 2
"#,
    )
    .expect("write mock");
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(bin_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(bin_path, perms).expect("chmod");
    }
}

fn write_settings(home: &Path) {
    let workspace = home.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(home.join(".direclaw")).expect("config dir");
    fs::write(
        home.join(".direclaw/config.yaml"),
        format!(
            r#"
workspace_path: {workspace}
shared_workspaces: {{}}
orchestrators: {{}}
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
auth_sync:
  enabled: true
  sources:
    codex:
      backend: onepassword
      reference: op://direclaw/codex-auth
      destination: "~/.codex/auth.json"
      owner_only: true
"#,
            workspace = workspace.display()
        ),
    )
    .expect("settings");
}

#[test]
fn auth_sync_command_fetches_and_persists_from_onepassword() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().to_path_buf();
    write_settings(&home);

    let bin_dir = home.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_op_mock(&bin_dir.join("op"));

    let output = run(&home, &["auth", "sync"], Some(&bin_dir), Some("svc-token"));
    assert_ok(&output);
    assert!(stdout(&output).contains("auth sync complete"));

    let destination = home.join(".codex/auth.json");
    let content = fs::read_to_string(&destination).expect("read synced auth");
    assert_eq!(content, "{\"token\":\"abc123\"}");

    #[cfg(unix)]
    {
        let mode = fs::metadata(&destination)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn auth_sync_requires_service_token() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().to_path_buf();
    write_settings(&home);

    let bin_dir = home.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_op_mock(&bin_dir.join("op"));

    let output = run(&home, &["auth", "sync"], Some(&bin_dir), None);
    assert_err_contains(
        &output,
        "OP_SERVICE_ACCOUNT_TOKEN is required for auth sync",
    );
}

#[test]
fn start_runs_auth_sync_before_marking_running() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().to_path_buf();
    write_settings(&home);

    let bin_dir = home.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_op_mock(&bin_dir.join("op"));

    let output = run(&home, &["start"], Some(&bin_dir), Some("svc-token"));
    assert_ok(&output);
    assert!(stdout(&output).contains("started"));
    assert!(stdout(&output).contains("auth_sync=synced(codex)"));
    assert!(home.join(".codex/auth.json").exists());
}
