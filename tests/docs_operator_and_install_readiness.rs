use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

#[test]
fn user_guide_entrypoint_covers_install_first_run_and_integrations() {
    let root = repo_root();
    let guide = read(&root.join("docs/user-guide/README.md"));

    for required in [
        "## Navigation",
        "## Install and First Run",
        "### 1. Download and install the release binary",
        "### 6. Start runtime and validate first message flow",
        "[Slack Setup](slack-setup.md)",
        "[Provider Auth Sync (1Password)](provider-auth-sync-1password.md)",
        "[Operator Runbook](operator-runbook.md)",
        "## Troubleshooting Matrix",
    ] {
        assert!(
            guide.contains(required),
            "docs/user-guide/README.md missing `{required}`"
        );
    }
}

#[test]
fn operator_runbook_includes_service_logs_backup_incident_and_upgrade_procedures() {
    let root = repo_root();
    let runbook = read(&root.join("docs/user-guide/operator-runbook.md"));

    for required in [
        "### Linux (`systemd`)",
        "### macOS (`launchd`)",
        "/etc/systemd/system/direclaw.service",
        "/Library/LaunchDaemons/com.direclaw.runtime.plist",
        "~/.direclaw/logs/security.log",
        "## Backup Strategy",
        "## Incident Procedures",
        "## Upgrade and Rollback",
        "install -m 0755 direclaw /usr/local/bin/direclaw",
    ] {
        assert!(
            runbook.contains(required),
            "docs/user-guide/operator-runbook.md missing `{required}`"
        );
    }
}

#[test]
fn governance_files_exist_non_empty_and_are_linked_from_readme() {
    let root = repo_root();
    for file in ["LICENSE", "CHANGELOG.md", "CONTRIBUTING.md", "SECURITY.md"] {
        let path = root.join(file);
        let metadata =
            fs::metadata(&path).unwrap_or_else(|e| panic!("{} missing: {e}", path.display()));
        assert!(metadata.len() > 0, "{} must be non-empty", path.display());
    }

    let readme = read(&root.join("README.md"));
    for link in [
        "[`LICENSE`](LICENSE)",
        "[`CHANGELOG.md`](CHANGELOG.md)",
        "[`CONTRIBUTING.md`](CONTRIBUTING.md)",
        "[`SECURITY.md`](SECURITY.md)",
    ] {
        assert!(
            readme.contains(link),
            "README.md missing governance link `{link}`"
        );
    }
}

#[test]
fn docs_clean_install_smoke_script_runs_in_ci_like_environment() {
    let root = repo_root();
    let script = root.join("scripts/ci/docs-clean-install-smoke.sh");
    assert!(script.exists(), "missing script: {}", script.display());

    let output = Command::new("bash")
        .arg(&script)
        .arg(env!("CARGO_BIN_EXE_direclaw"))
        .output()
        .expect("run docs clean install smoke script");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("docs_clean_install_smoke=ok"));
}
