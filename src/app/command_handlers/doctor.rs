use crate::app::command_support::{load_settings, map_config_err};
use crate::channels::slack;
use crate::config::{
    default_global_config_path, load_orchestrator_config, OrchestratorConfig, Settings,
};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
struct DoctorFinding {
    id: String,
    ok: bool,
    detail: String,
    remediation: String,
}

fn doctor_finding(
    id: impl Into<String>,
    ok: bool,
    detail: impl Into<String>,
    remediation: impl Into<String>,
) -> DoctorFinding {
    DoctorFinding {
        id: id.into(),
        ok,
        detail: detail.into(),
        remediation: remediation.into(),
    }
}

fn load_orchestrator_or_err(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<OrchestratorConfig, String> {
    load_orchestrator_config(settings, orchestrator_id).map_err(map_config_err)
}

fn is_binary_available(binary: &str) -> bool {
    if binary.trim().is_empty() {
        return false;
    }
    let explicit = Path::new(binary);
    if explicit.components().count() > 1 || explicit.is_absolute() {
        return is_executable_file(explicit);
    }

    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(binary);
        if is_executable_file(&candidate) {
            return true;
        }
        #[cfg(windows)]
        {
            if is_executable_file(&dir.join(format!("{binary}.exe"))) {
                return true;
            }
        }
        false
    })
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn now_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

fn can_write_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    let probe = path.join(format!(".direclaw-doctor-{}", now_nanos()));
    fs::write(&probe, b"ok").map_err(|e| format!("failed to write {}: {e}", probe.display()))?;
    fs::remove_file(&probe).map_err(|e| format!("failed to remove {}: {e}", probe.display()))
}

pub fn cmd_doctor() -> Result<String, String> {
    let mut findings = Vec::new();
    let config_path = default_global_config_path().map_err(map_config_err)?;
    findings.push(doctor_finding(
        "config.path",
        config_path.exists(),
        format!("config={}", config_path.display()),
        "run `direclaw setup` to create default config",
    ));

    let settings = match load_settings() {
        Ok(settings) => {
            findings.push(doctor_finding(
                "config.parse",
                true,
                "settings parsed and validated",
                "none",
            ));
            Some(settings)
        }
        Err(err) => {
            findings.push(doctor_finding(
                "config.parse",
                false,
                format!("settings load failed: {err}"),
                "fix ~/.direclaw/config.yaml and retry `direclaw doctor`",
            ));
            None
        }
    };

    let anthropic_bin = std::env::var("DIRECLAW_PROVIDER_BIN_ANTHROPIC")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "claude".to_string());
    let openai_bin = std::env::var("DIRECLAW_PROVIDER_BIN_OPENAI")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "codex".to_string());
    findings.push(doctor_finding(
        "binary.anthropic",
        is_binary_available(&anthropic_bin),
        format!("binary={anthropic_bin}"),
        "install the Anthropic CLI or set DIRECLAW_PROVIDER_BIN_ANTHROPIC",
    ));
    findings.push(doctor_finding(
        "binary.openai",
        is_binary_available(&openai_bin),
        format!("binary={openai_bin}"),
        "install the OpenAI Codex CLI or set DIRECLAW_PROVIDER_BIN_OPENAI",
    ));

    if let Some(settings) = settings.as_ref() {
        findings.push(match can_write_directory(&settings.workspaces_path) {
            Ok(_) => doctor_finding(
                "workspace.root",
                true,
                format!("writable={}", settings.workspaces_path.display()),
                "none",
            ),
            Err(err) => doctor_finding(
                "workspace.root",
                false,
                err,
                "grant write permission to workspaces_path in ~/.direclaw/config.yaml",
            ),
        });
        for orchestrator_id in settings.orchestrators.keys() {
            match settings.resolve_private_workspace(orchestrator_id) {
                Ok(private_workspace) => {
                    findings.push(match can_write_directory(&private_workspace) {
                        Ok(_) => doctor_finding(
                            format!("workspace.orchestrator.{orchestrator_id}"),
                            true,
                            format!("writable={}", private_workspace.display()),
                            "none",
                        ),
                        Err(err) => doctor_finding(
                            format!("workspace.orchestrator.{orchestrator_id}"),
                            false,
                            err,
                            format!(
                                "grant write permission or update `orchestrators.{orchestrator_id}.private_workspace`"
                            ),
                        ),
                    });
                    findings.push(doctor_finding(
                        format!("config.orchestrator.{orchestrator_id}"),
                        load_orchestrator_or_err(settings, orchestrator_id).is_ok(),
                        format!(
                            "source={}",
                            private_workspace.join("orchestrator.yaml").display()
                        ),
                        format!(
                            "run `direclaw orchestrator add {orchestrator_id}` or create {}/orchestrator.yaml",
                            private_workspace.display()
                        ),
                    ));
                }
                Err(err) => findings.push(doctor_finding(
                    format!("workspace.orchestrator.{orchestrator_id}"),
                    false,
                    err.to_string(),
                    "fix orchestrator private workspace config",
                )),
            }
        }

        if settings.auth_sync.enabled {
            let token_ok = std::env::var("OP_SERVICE_ACCOUNT_TOKEN")
                .ok()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
            findings.push(doctor_finding(
                "env.OP_SERVICE_ACCOUNT_TOKEN",
                token_ok,
                "required for auth sync",
                "set OP_SERVICE_ACCOUNT_TOKEN before running `direclaw auth sync`",
            ));
            findings.push(doctor_finding(
                "binary.op",
                is_binary_available("op"),
                "required for auth sync backend onepassword",
                "install 1Password CLI (`op`) and ensure PATH includes it",
            ));
        }

        if settings
            .channels
            .get("slack")
            .map(|cfg| cfg.enabled)
            .unwrap_or(false)
        {
            findings.push(match slack::validate_startup_credentials(settings) {
                Ok(_) => doctor_finding("env.slack", true, "slack credentials validated", "none"),
                Err(err) => doctor_finding(
                    "env.slack",
                    false,
                    err.to_string(),
                    "set required SLACK_* env vars for each configured slack profile",
                ),
            });
        }
    }

    let failed = findings.iter().filter(|f| !f.ok).count();
    let summary = if failed == 0 { "healthy" } else { "unhealthy" };
    let mut lines = vec![
        format!("summary={summary}"),
        format!("checks_total={}", findings.len()),
        format!("checks_failed={failed}"),
    ];
    for finding in findings {
        lines.push(format!(
            "check:{}={}",
            finding.id,
            if finding.ok { "ok" } else { "fail" }
        ));
        lines.push(format!("check:{}.detail={}", finding.id, finding.detail));
        if !finding.ok {
            lines.push(format!(
                "check:{}.remediation={}",
                finding.id, finding.remediation
            ));
        }
    }
    Ok(lines.join("\n"))
}
