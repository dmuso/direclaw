pub use super::orchestrators_registry::{remove_orchestrator_config, save_orchestrator_registry};
use super::{
    default_global_config_path, ConfigError, OrchestratorConfig, Settings, ValidationOptions,
};
use crate::memory::{bootstrap_memory_paths_for_runtime_root, MemoryPathError};
use crate::orchestration::shared_mounts::{
    reconcile_all_orchestrator_shared_mounts, reconcile_orchestrator_shared_mounts,
};
use crate::prompts::ensure_orchestrator_prompt_templates;
use std::fs;
use std::path::{Path, PathBuf};

fn create_parent_dir(path: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigError::CreateDir {
            path: parent.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

pub fn save_settings(settings: &Settings) -> Result<PathBuf, ConfigError> {
    settings.validate(ValidationOptions {
        require_shared_paths_exist: false,
    })?;

    let path = default_global_config_path()?;
    create_parent_dir(&path)?;
    let body = serde_yaml::to_string(settings).map_err(|source| ConfigError::Encode {
        path: path.display().to_string(),
        source,
    })?;
    fs::write(&path, body).map_err(|source| ConfigError::Write {
        path: path.display().to_string(),
        source,
    })?;
    reconcile_all_orchestrator_shared_mounts(settings).map_err(|err| {
        ConfigError::Settings(format!(
            "failed to reconcile shared workspace mounts after saving settings: {err}"
        ))
    })?;
    Ok(path)
}

pub fn save_orchestrator_config(
    settings: &Settings,
    orchestrator_id: &str,
    orchestrator: &OrchestratorConfig,
) -> Result<PathBuf, ConfigError> {
    orchestrator.validate(settings, orchestrator_id)?;
    let private_workspace = settings.resolve_private_workspace(orchestrator_id)?;
    fs::create_dir_all(&private_workspace).map_err(|source| ConfigError::CreateDir {
        path: private_workspace.display().to_string(),
        source,
    })?;
    bootstrap_memory_paths_for_runtime_root(&private_workspace).map_err(|err| match err {
        MemoryPathError::Canonicalize { path, source }
        | MemoryPathError::CreateDir { path, source } => ConfigError::CreateDir { path, source },
    })?;
    ensure_orchestrator_prompt_templates(&private_workspace, orchestrator)?;
    let path = private_workspace.join("orchestrator.yaml");
    let body = serde_yaml::to_string(orchestrator).map_err(|source| ConfigError::Encode {
        path: path.display().to_string(),
        source,
    })?;
    fs::write(&path, body).map_err(|source| ConfigError::Write {
        path: path.display().to_string(),
        source,
    })?;
    reconcile_orchestrator_shared_mounts(settings, orchestrator_id).map_err(|err| {
        ConfigError::Settings(format!(
            "failed to reconcile shared workspace mounts for orchestrator `{orchestrator_id}`: {err}"
        ))
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn save_settings_reconciles_shared_workspace_symlink_mounts() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let temp = tempdir().expect("tempdir");
        let _home_guard = HomeGuard::set(temp.path());

        let private_workspace = temp.path().join("workspaces").join("alpha");
        let shared_docs = temp.path().join("shared").join("docs");
        fs::create_dir_all(&shared_docs).expect("create shared docs");
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
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
            temp.path().join("workspaces").display(),
            shared_docs.display(),
            private_workspace.display()
        ))
        .expect("parse settings");

        save_settings(&settings).expect("save settings");

        assert_symlink_to(&private_workspace.join("shared/docs"), &shared_docs);
    }

    #[test]
    fn save_orchestrator_config_repairs_wrong_mount_target() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let temp = tempdir().expect("tempdir");
        let _home_guard = HomeGuard::set(temp.path());

        let private_workspace = temp.path().join("workspaces").join("alpha");
        let shared_docs = temp.path().join("shared").join("docs");
        let shared_old = temp.path().join("shared").join("old");
        fs::create_dir_all(&shared_docs).expect("create shared docs");
        fs::create_dir_all(&shared_old).expect("create shared old");
        fs::create_dir_all(private_workspace.join("shared")).expect("create mount root");
        create_symlink(&shared_old, &private_workspace.join("shared/docs")).expect("seed mount");

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
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
            temp.path().join("workspaces").display(),
            shared_docs.display(),
            private_workspace.display()
        ))
        .expect("parse settings");
        let orchestrator: OrchestratorConfig = serde_yaml::from_str(
            r#"
id: alpha
selector_agent: workflow_router
default_workflow: minimal_single_agent
selection_max_retries: 2
agents:
  workflow_router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
  default:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: false
workflows:
  - id: minimal_single_agent
    version: 1
    description: test workflow
    tags: [test]
    inputs: [user_prompt]
    steps:
      - id: generate
        type: agent_task
        agent: default
        prompt: hi
        outputs: [summary]
        output_files:
          summary: output/summary.txt
"#,
        )
        .expect("parse orchestrator");

        save_orchestrator_config(&settings, "alpha", &orchestrator).expect("save orchestrator");

        assert_symlink_to(&private_workspace.join("shared/docs"), &shared_docs);
    }

    fn assert_symlink_to(link: &Path, target: &Path) {
        let metadata = fs::symlink_metadata(link).expect("link metadata");
        assert!(metadata.file_type().is_symlink());
        assert_eq!(fs::read_link(link).expect("read link"), target);
    }

    struct HomeGuard {
        old_home: Option<std::ffi::OsString>,
    }

    impl HomeGuard {
        fn set(home: &Path) -> Self {
            let old_home = std::env::var_os("HOME");
            std::env::set_var("HOME", home);
            Self { old_home }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            if let Some(old_home) = self.old_home.take() {
                std::env::set_var("HOME", old_home);
            } else {
                std::env::remove_var("HOME");
            }
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
}
