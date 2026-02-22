use crate::config::Settings;
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::workspace_access::normalize_absolute_path;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub fn reconcile_all_orchestrator_shared_mounts(
    settings: &Settings,
) -> Result<(), OrchestratorError> {
    for orchestrator_id in settings.orchestrators.keys() {
        reconcile_orchestrator_shared_mounts(settings, orchestrator_id)?;
    }
    Ok(())
}

pub fn reconcile_orchestrator_shared_mounts(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<(), OrchestratorError> {
    let private_workspace = settings.resolve_private_workspace(orchestrator_id)?;
    let private_workspace = normalize_absolute_path(&private_workspace)?;
    fs::create_dir_all(&private_workspace).map_err(|err| io_error(&private_workspace, err))?;

    let orchestrator = settings.orchestrators.get(orchestrator_id).ok_or_else(|| {
        OrchestratorError::Config(format!(
            "missing orchestrator `{orchestrator_id}` in settings"
        ))
    })?;
    let mount_root = private_workspace.join("shared");
    fs::create_dir_all(&mount_root).map_err(|err| io_error(&mount_root, err))?;

    let mut desired: BTreeMap<String, PathBuf> = BTreeMap::new();
    for key in &orchestrator.shared_access {
        let target = settings.shared_workspaces.get(key).ok_or_else(|| {
            OrchestratorError::Config(format!(
                "orchestrator `{orchestrator_id}` references unknown shared workspace `{key}`"
            ))
        })?;
        let canonical_target = fs::canonicalize(&target.path).map_err(|_| {
            OrchestratorError::Config(format!(
                "shared workspace `{key}` path `{}` is missing or invalid",
                target.path.display()
            ))
        })?;
        desired.insert(key.clone(), normalize_absolute_path(&canonical_target)?);
    }

    for (key, target) in &desired {
        let mount_path = mount_root.join(key);
        reconcile_mount_path(orchestrator_id, &mount_path, target)?;
    }
    remove_stale_symlink_mounts(&mount_root, &desired)?;
    Ok(())
}

fn reconcile_mount_path(
    orchestrator_id: &str,
    mount_path: &Path,
    desired_target: &Path,
) -> Result<(), OrchestratorError> {
    if !mount_path.exists() {
        create_symlink(desired_target, mount_path).map_err(|err| io_error(mount_path, err))?;
        return Ok(());
    }
    let metadata = fs::symlink_metadata(mount_path).map_err(|err| io_error(mount_path, err))?;
    if !metadata.file_type().is_symlink() {
        return Err(OrchestratorError::Config(format!(
            "orchestrator `{orchestrator_id}` shared mount `{}` exists but existing mount path is not a symlink",
            mount_path.display()
        )));
    }

    let points_to_desired = fs::canonicalize(mount_path)
        .ok()
        .and_then(|actual| normalize_absolute_path(&actual).ok())
        .map(|actual| actual == desired_target)
        .unwrap_or(false);
    if points_to_desired {
        return Ok(());
    }

    remove_symlink(mount_path).map_err(|err| io_error(mount_path, err))?;
    create_symlink(desired_target, mount_path).map_err(|err| io_error(mount_path, err))
}

fn remove_stale_symlink_mounts(
    mount_root: &Path,
    desired: &BTreeMap<String, PathBuf>,
) -> Result<(), OrchestratorError> {
    for entry in fs::read_dir(mount_root).map_err(|err| io_error(mount_root, err))? {
        let entry = entry.map_err(|err| io_error(mount_root, err))?;
        let file_name = entry.file_name();
        if desired.contains_key(os_str_to_string(&file_name).as_str()) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|err| io_error(&path, err))?;
        if metadata.file_type().is_symlink() {
            remove_symlink(&path).map_err(|err| io_error(&path, err))?;
        }
    }
    Ok(())
}

fn os_str_to_string(value: &OsStr) -> String {
    value.to_string_lossy().to_string()
}

fn io_error(path: &Path, source: std::io::Error) -> OrchestratorError {
    OrchestratorError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

fn remove_symlink(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::IsADirectory => fs::remove_dir(path),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn reconcile_creates_missing_shared_symlink_mount() {
        let temp = tempdir().expect("tempdir");
        let private = temp.path().join("workspaces").join("alpha");
        let shared_docs = temp.path().join("shared").join("docs");
        fs::create_dir_all(&shared_docs).expect("create docs");
        let settings = parse_settings(&private, &[("docs", &shared_docs)], &["docs"]);

        reconcile_orchestrator_shared_mounts(&settings, "alpha").expect("reconcile");

        let link = private.join("shared/docs");
        assert!(link.exists());
        assert_symlink_to(&link, &shared_docs);
    }

    #[test]
    fn reconcile_repairs_wrong_target_symlink() {
        let temp = tempdir().expect("tempdir");
        let private = temp.path().join("workspaces").join("alpha");
        let shared_docs = temp.path().join("shared").join("docs");
        let shared_old = temp.path().join("shared").join("old");
        fs::create_dir_all(&shared_docs).expect("create docs");
        fs::create_dir_all(&shared_old).expect("create old");
        fs::create_dir_all(private.join("shared")).expect("create shared root");
        let link = private.join("shared/docs");
        create_symlink(&shared_old, &link).expect("seed wrong symlink");

        let settings = parse_settings(&private, &[("docs", &shared_docs)], &["docs"]);
        reconcile_orchestrator_shared_mounts(&settings, "alpha").expect("reconcile");

        assert_symlink_to(&link, &shared_docs);
    }

    #[test]
    fn reconcile_removes_stale_symlink_mounts() {
        let temp = tempdir().expect("tempdir");
        let private = temp.path().join("workspaces").join("alpha");
        let shared_docs = temp.path().join("shared").join("docs");
        let shared_old = temp.path().join("shared").join("old");
        fs::create_dir_all(&shared_docs).expect("create docs");
        fs::create_dir_all(&shared_old).expect("create old");
        fs::create_dir_all(private.join("shared")).expect("create shared root");
        let stale = private.join("shared/stale");
        create_symlink(&shared_old, &stale).expect("seed stale symlink");

        let settings = parse_settings(&private, &[("docs", &shared_docs)], &["docs"]);
        reconcile_orchestrator_shared_mounts(&settings, "alpha").expect("reconcile");

        assert!(!stale.exists(), "stale symlink should be removed");
        assert_symlink_to(&private.join("shared/docs"), &shared_docs);
    }

    #[test]
    fn reconcile_fails_when_mount_path_is_not_symlink() {
        let temp = tempdir().expect("tempdir");
        let private = temp.path().join("workspaces").join("alpha");
        let shared_docs = temp.path().join("shared").join("docs");
        fs::create_dir_all(&shared_docs).expect("create docs");
        fs::create_dir_all(private.join("shared/docs")).expect("seed colliding directory");
        let settings = parse_settings(&private, &[("docs", &shared_docs)], &["docs"]);

        let err = reconcile_orchestrator_shared_mounts(&settings, "alpha").expect_err("must fail");
        assert!(
            err.to_string()
                .contains("existing mount path is not a symlink"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reconcile_all_processes_each_orchestrator() {
        let temp = tempdir().expect("tempdir");
        let alpha_private = temp.path().join("workspaces").join("alpha");
        let beta_private = temp.path().join("workspaces").join("beta");
        let shared_docs = temp.path().join("shared").join("docs");
        fs::create_dir_all(&shared_docs).expect("create docs");
        let settings: Settings = serde_yaml::from_str(&format!(
            r#"
workspaces_path: {}
shared_workspaces:
  docs:
    path: {}
    description: shared docs
orchestrators:
  alpha:
    private_workspace: {}
    shared_access: [docs]
  beta:
    private_workspace: {}
    shared_access: [docs]
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
            temp.path().join("workspaces").display(),
            shared_docs.display(),
            alpha_private.display(),
            beta_private.display()
        ))
        .expect("parse settings");

        reconcile_all_orchestrator_shared_mounts(&settings).expect("reconcile");

        assert_symlink_to(&alpha_private.join("shared/docs"), &shared_docs);
        assert_symlink_to(&beta_private.join("shared/docs"), &shared_docs);
    }

    fn parse_settings(
        private_workspace: &Path,
        shared: &[(&str, &Path)],
        grants: &[&str],
    ) -> Settings {
        let mut shared_yaml = String::new();
        for (key, path) in shared {
            shared_yaml.push_str(&format!(
                "  {key}:\n    path: {}\n    description: shared workspace {key}\n",
                path.display()
            ));
        }
        let grants_yaml = if grants.is_empty() {
            "[]".to_string()
        } else {
            format!("[{}]", grants.join(", "))
        };
        serde_yaml::from_str(&format!(
            r#"
workspaces_path: {}
shared_workspaces:
{}
orchestrators:
  alpha:
    private_workspace: {}
    shared_access: {}
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
            private_workspace
                .parent()
                .expect("private workspace parent")
                .display(),
            shared_yaml,
            private_workspace.display(),
            grants_yaml
        ))
        .expect("parse settings")
    }

    fn assert_symlink_to(link: &Path, target: &Path) {
        let metadata = fs::symlink_metadata(link).expect("link metadata");
        assert!(metadata.file_type().is_symlink());
        let actual = fs::read_link(link).expect("read link");
        assert_eq!(actual, target);
    }

    #[cfg(unix)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }
}
