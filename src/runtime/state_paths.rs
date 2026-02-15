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
            self.root.join("queue/incoming"),
            self.root.join("queue/processing"),
            self.root.join("queue/outgoing"),
            self.root.join("files"),
            self.root.join("logs"),
            self.root.join("orchestrator/messages"),
            self.root.join("orchestrator/select/incoming"),
            self.root.join("orchestrator/select/processing"),
            self.root.join("orchestrator/select/results"),
            self.root.join("orchestrator/select/logs"),
            self.root.join("orchestrator/diagnostics/incoming"),
            self.root.join("orchestrator/diagnostics/processing"),
            self.root.join("orchestrator/diagnostics/context"),
            self.root.join("orchestrator/diagnostics/results"),
            self.root.join("orchestrator/diagnostics/logs"),
            self.root.join("workflows/runs"),
            self.root.join("channels"),
            self.root.join("daemon"),
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
