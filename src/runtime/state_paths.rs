use super::RuntimeError;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePaths {
    pub root: PathBuf,
}

impl StatePaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn required_directories(&self) -> Vec<PathBuf> {
        vec![
            self.root.join("files"),
            self.root.join("logs"),
            self.root.join("channels"),
            self.root.join("daemon"),
            self.root.join("runtime"),
        ]
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("config.yaml")
    }

    pub fn daemon_dir(&self) -> PathBuf {
        self.root.join("daemon")
    }

    pub fn supervisor_state_path(&self) -> PathBuf {
        self.daemon_dir().join("runtime.json")
    }

    pub fn supervisor_lock_path(&self) -> PathBuf {
        self.daemon_dir().join("supervisor.lock")
    }

    pub fn stop_signal_path(&self) -> PathBuf {
        self.daemon_dir().join("stop")
    }

    pub fn runtime_log_path(&self) -> PathBuf {
        self.root.join("logs/runtime.log")
    }

    pub fn orchestrator_log_path(&self) -> PathBuf {
        self.root.join("logs/orchestrator.log")
    }
}

pub const DEFAULT_STATE_ROOT_DIR: &str = ".direclaw";

pub fn default_state_root_path() -> Result<PathBuf, RuntimeError> {
    let home = std::env::var_os("HOME").ok_or(RuntimeError::HomeDirectoryUnavailable)?;
    Ok(PathBuf::from(home).join(DEFAULT_STATE_ROOT_DIR))
}

pub fn bootstrap_state_root(paths: &StatePaths) -> Result<(), RuntimeError> {
    for path in paths.required_directories() {
        fs::create_dir_all(&path).map_err(|source| RuntimeError::CreateDir {
            path: path.display().to_string(),
            source,
        })?;
    }
    Ok(())
}
