use crate::config::Settings;
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::workspace_access::normalize_absolute_path;
use std::fs;
use std::path::Path;

pub fn reconcile_all_orchestrator_skill_mounts(
    settings: &Settings,
) -> Result<(), OrchestratorError> {
    for orchestrator_id in settings.orchestrators.keys() {
        reconcile_orchestrator_skill_mounts(settings, orchestrator_id)?;
    }
    Ok(())
}

pub fn reconcile_orchestrator_skill_mounts(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<(), OrchestratorError> {
    let private_workspace = settings.resolve_private_workspace(orchestrator_id)?;
    let private_workspace = normalize_absolute_path(&private_workspace)?;
    fs::create_dir_all(&private_workspace).map_err(|err| io_error(&private_workspace, err))?;

    let skills_root = private_workspace.join("skills");
    fs::create_dir_all(&skills_root).map_err(|err| io_error(&skills_root, err))?;

    let agents_root = private_workspace.join(".agents");
    fs::create_dir_all(&agents_root).map_err(|err| io_error(&agents_root, err))?;
    reconcile_mount_path(
        orchestrator_id,
        "agent",
        &agents_root.join("skills"),
        &skills_root,
    )?;

    let claude_root = private_workspace.join(".claude");
    fs::create_dir_all(&claude_root).map_err(|err| io_error(&claude_root, err))?;
    reconcile_mount_path(
        orchestrator_id,
        "claude",
        &claude_root.join("skills"),
        &skills_root,
    )
}

fn reconcile_mount_path(
    orchestrator_id: &str,
    mount_kind: &str,
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
            "orchestrator `{orchestrator_id}` {mount_kind} skills mount `{}` exists but is not a symlink",
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
    fn reconcile_creates_skills_root_and_mount_symlinks() {
        let temp = tempdir().expect("tempdir");
        let private = temp.path().join("workspaces").join("alpha");
        let settings = parse_settings(&private);

        reconcile_orchestrator_skill_mounts(&settings, "alpha").expect("reconcile");

        assert!(private.join("skills").is_dir());
        assert_symlink_to(&private.join(".agents/skills"), &private.join("skills"));
        assert_symlink_to(&private.join(".claude/skills"), &private.join("skills"));
    }

    #[test]
    fn reconcile_repairs_wrong_target_symlink() {
        let temp = tempdir().expect("tempdir");
        let private = temp.path().join("workspaces").join("alpha");
        fs::create_dir_all(private.join(".agents")).expect("create .agents");
        fs::create_dir_all(private.join(".claude")).expect("create .claude");
        fs::create_dir_all(private.join("skills")).expect("create skills");
        let old_target = temp.path().join("old-skills");
        fs::create_dir_all(&old_target).expect("create old target");
        create_symlink(&old_target, &private.join(".agents/skills")).expect("seed bad .agents");
        create_symlink(&old_target, &private.join(".claude/skills")).expect("seed bad .claude");

        let settings = parse_settings(&private);
        reconcile_orchestrator_skill_mounts(&settings, "alpha").expect("reconcile");

        assert_symlink_to(&private.join(".agents/skills"), &private.join("skills"));
        assert_symlink_to(&private.join(".claude/skills"), &private.join("skills"));
    }

    #[test]
    fn reconcile_fails_when_existing_mount_path_is_not_symlink() {
        let temp = tempdir().expect("tempdir");
        let private = temp.path().join("workspaces").join("alpha");
        fs::create_dir_all(private.join(".agents/skills")).expect("seed non-symlink path");
        let settings = parse_settings(&private);

        let err = reconcile_orchestrator_skill_mounts(&settings, "alpha").expect_err("must fail");
        assert!(
            err.to_string().contains("is not a symlink"),
            "unexpected error: {err}"
        );
    }

    fn parse_settings(private_workspace: &Path) -> Settings {
        serde_yaml::from_str(&format!(
            r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators:
  alpha:
    private_workspace: {}
    shared_access: []
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
            private_workspace
                .parent()
                .expect("private workspace parent")
                .display(),
            private_workspace.display()
        ))
        .expect("parse settings")
    }

    fn assert_symlink_to(link: &Path, target: &Path) {
        let metadata = fs::symlink_metadata(link).expect("link metadata");
        assert!(metadata.file_type().is_symlink());
        assert_eq!(fs::read_link(link).expect("read link"), target);
    }
}
