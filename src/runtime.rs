use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingDefaults {
    pub queue_poll_interval_secs: u64,
    pub outbound_poll_interval_secs: u64,
}

impl Default for PollingDefaults {
    fn default() -> Self {
        Self {
            queue_poll_interval_secs: 1,
            outbound_poll_interval_secs: 1,
        }
    }
}

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
        ]
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.yaml")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("failed to create runtime path {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WorkerKind {
    QueueProcessor,
    Orchestrator,
    ChannelAdapter(String),
    Heartbeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    Stopped,
    Running,
}

#[derive(Debug, Default)]
pub struct WorkerRegistry {
    workers: HashMap<WorkerKind, WorkerState>,
}

impl WorkerRegistry {
    pub fn register(&mut self, worker: WorkerKind) {
        self.workers.entry(worker).or_insert(WorkerState::Stopped);
    }

    pub fn start(&mut self, worker: &WorkerKind) {
        if let Some(state) = self.workers.get_mut(worker) {
            *state = WorkerState::Running;
        }
    }

    pub fn stop(&mut self, worker: &WorkerKind) {
        if let Some(state) = self.workers.get_mut(worker) {
            *state = WorkerState::Stopped;
        }
    }

    pub fn state(&self, worker: &WorkerKind) -> Option<WorkerState> {
        self.workers.get(worker).copied()
    }

    pub fn all(&self) -> &HashMap<WorkerKind, WorkerState> {
        &self.workers
    }
}

pub fn canonicalize_existing(path: &Path) -> Result<PathBuf, std::io::Error> {
    fs::canonicalize(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn polling_defaults_are_one_second() {
        let defaults = PollingDefaults::default();
        assert_eq!(defaults.queue_poll_interval_secs, 1);
        assert_eq!(defaults.outbound_poll_interval_secs, 1);
    }

    #[test]
    fn bootstrap_creates_required_directories() {
        let dir = tempdir().expect("temp dir");
        let paths = StatePaths::new(dir.path().join("state"));
        bootstrap_state_root(&paths).expect("bootstrap succeeds");

        for required in paths.required_directories() {
            assert!(
                required.is_dir(),
                "missing directory: {}",
                required.display()
            );
        }
    }

    #[test]
    fn worker_registry_tracks_independent_lifecycle() {
        let mut registry = WorkerRegistry::default();
        let queue = WorkerKind::QueueProcessor;
        let orchestrator = WorkerKind::Orchestrator;
        let slack = WorkerKind::ChannelAdapter("slack".to_string());
        let heartbeat = WorkerKind::Heartbeat;

        registry.register(queue.clone());
        registry.register(orchestrator.clone());
        registry.register(slack.clone());
        registry.register(heartbeat.clone());

        registry.start(&queue);
        registry.start(&slack);

        assert_eq!(registry.state(&queue), Some(WorkerState::Running));
        assert_eq!(registry.state(&orchestrator), Some(WorkerState::Stopped));
        assert_eq!(registry.state(&slack), Some(WorkerState::Running));
        assert_eq!(registry.state(&heartbeat), Some(WorkerState::Stopped));

        registry.stop(&slack);
        assert_eq!(registry.state(&slack), Some(WorkerState::Stopped));
    }
}
