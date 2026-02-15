use crate::config::{default_global_config_path, AuthSyncSource, Settings, ValidationOptions};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub(crate) struct AuthSyncResult {
    pub(crate) synced_sources: Vec<String>,
}

pub(crate) fn render_auth_sync_result(result: &AuthSyncResult, command_context: bool) -> String {
    if result.synced_sources.is_empty() {
        if command_context {
            return "auth sync skipped\nauth_sync_enabled=false".to_string();
        }
        return "auth_sync=skipped".to_string();
    }

    if command_context {
        return format!(
            "auth sync complete\nsynced={}\nsources={}",
            result.synced_sources.len(),
            result.synced_sources.join(",")
        );
    }
    format!("auth_sync=synced({})", result.synced_sources.join(","))
}

pub fn cmd_auth(args: &[String]) -> Result<String, String> {
    if args.len() == 1 && args[0] == "sync" {
        let settings = load_settings_for_auth()?;
        let result = sync_auth_sources(&settings)?;
        return Ok(render_auth_sync_result(&result, true));
    }
    Err("usage: auth sync".to_string())
}

fn load_settings_for_auth() -> Result<Settings, String> {
    let path = default_global_config_path().map_err(|e| e.to_string())?;
    let settings = Settings::from_path(&path).map_err(|e| e.to_string())?;
    settings
        .validate(ValidationOptions {
            require_shared_paths_exist: false,
        })
        .map_err(|e| e.to_string())?;
    Ok(settings)
}

fn resolve_auth_destination(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is unavailable".to_string())?;
        return Ok(home.join(rest));
    }
    Err(format!(
        "auth destination `{}` must be absolute or start with `~/`",
        path.display()
    ))
}

fn op_service_token() -> Result<String, String> {
    let raw = std::env::var("OP_SERVICE_ACCOUNT_TOKEN")
        .map_err(|_| "OP_SERVICE_ACCOUNT_TOKEN is required for auth sync".to_string())?;
    if raw.trim().is_empty() {
        return Err("OP_SERVICE_ACCOUNT_TOKEN is required for auth sync".to_string());
    }
    Ok(raw)
}

fn sync_onepassword_source(
    source_id: &str,
    source: &AuthSyncSource,
    token: &str,
) -> Result<(), String> {
    let destination = resolve_auth_destination(&source.destination)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }

    let output = Command::new("op")
        .arg("read")
        .arg(&source.reference)
        .env("OP_SERVICE_ACCOUNT_TOKEN", token)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "auth sync failed: `op` binary is not available in PATH".to_string()
            } else {
                format!("auth sync source `{source_id}` failed to execute op: {e}")
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.trim();
        if reason.is_empty() {
            return Err(format!(
                "auth sync source `{source_id}` failed to read `{}`",
                source.reference
            ));
        }
        return Err(format!(
            "auth sync source `{source_id}` failed to read `{}`: {}",
            source.reference, reason
        ));
    }

    if output.stdout.is_empty() {
        return Err(format!(
            "auth sync source `{source_id}` returned empty content"
        ));
    }

    let file_name = destination
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| {
            format!(
                "auth sync source `{source_id}` destination `{}` must include a file name",
                destination.display()
            )
        })?;
    let temp_path = destination.with_file_name(format!(".{file_name}.tmp-{}", now_nanos()));
    fs::write(&temp_path, &output.stdout)
        .map_err(|e| format!("failed to write {}: {e}", temp_path.display()))?;

    #[cfg(unix)]
    if source.owner_only {
        let mut perms = fs::metadata(&temp_path)
            .map_err(|e| format!("failed to read {}: {e}", temp_path.display()))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&temp_path, perms)
            .map_err(|e| format!("failed to chmod {}: {e}", temp_path.display()))?;
    }

    fs::rename(&temp_path, &destination).map_err(|e| {
        let _ = fs::remove_file(&temp_path);
        format!(
            "failed to replace {} with {}: {e}",
            destination.display(),
            temp_path.display()
        )
    })?;
    Ok(())
}

pub(crate) fn sync_auth_sources(settings: &Settings) -> Result<AuthSyncResult, String> {
    if !settings.auth_sync.enabled {
        return Ok(AuthSyncResult::default());
    }

    let mut result = AuthSyncResult::default();
    let token = op_service_token()?;

    for (source_id, source) in &settings.auth_sync.sources {
        match source.backend.trim() {
            "onepassword" => sync_onepassword_source(source_id, source, &token)?,
            other => {
                return Err(format!(
                    "auth sync source `{source_id}` has unsupported backend `{other}`"
                ));
            }
        }
        result.synced_sources.push(source_id.clone());
    }
    Ok(result)
}

fn now_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as i128)
        .unwrap_or(0)
}
