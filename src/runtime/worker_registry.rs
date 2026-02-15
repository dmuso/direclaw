use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Started {
        worker_id: String,
        at: i64,
    },
    Heartbeat {
        worker_id: String,
        at: i64,
    },
    Error {
        worker_id: String,
        at: i64,
        message: String,
        fatal: bool,
    },
    Stopped {
        worker_id: String,
        at: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WorkerKind {
    QueueProcessor,
    Orchestrator,
    ChannelAdapter(String),
    Heartbeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Stopped,
    Running,
    Error,
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

    pub fn fail(&mut self, worker: &WorkerKind) {
        if let Some(state) = self.workers.get_mut(worker) {
            *state = WorkerState::Error;
        }
    }

    pub fn state(&self, worker: &WorkerKind) -> Option<WorkerState> {
        self.workers.get(worker).copied()
    }

    pub fn all(&self) -> &HashMap<WorkerKind, WorkerState> {
        &self.workers
    }
}
