use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    Memory,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerHealth {
    pub state: WorkerState,
    pub last_heartbeat: Option<i64>,
    pub last_error: Option<String>,
}

impl Default for WorkerHealth {
    fn default() -> Self {
        Self {
            state: WorkerState::Stopped,
            last_heartbeat: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerEventLog {
    pub level: &'static str,
    pub event: &'static str,
    pub message: String,
}

pub fn apply_worker_event(
    workers: &mut BTreeMap<String, WorkerHealth>,
    active: &mut BTreeSet<String>,
    event: WorkerEvent,
) -> Option<WorkerEventLog> {
    match event {
        WorkerEvent::Started { worker_id, at } => {
            let entry = workers.entry(worker_id.clone()).or_default();
            entry.state = WorkerState::Running;
            entry.last_heartbeat = Some(at);
            Some(WorkerEventLog {
                level: "info",
                event: "worker.started",
                message: worker_id,
            })
        }
        WorkerEvent::Heartbeat { worker_id, at } => {
            let entry = workers.entry(worker_id).or_default();
            if entry.state != WorkerState::Error {
                entry.state = WorkerState::Running;
            }
            entry.last_heartbeat = Some(at);
            None
        }
        WorkerEvent::Error {
            worker_id,
            at,
            message,
            fatal,
        } => {
            let entry = workers.entry(worker_id.clone()).or_default();
            entry.state = WorkerState::Error;
            entry.last_heartbeat = Some(at);
            entry.last_error = Some(message.clone());
            Some(WorkerEventLog {
                level: if fatal { "error" } else { "warn" },
                event: "worker.error",
                message: format!("{}: {}", worker_id, message),
            })
        }
        WorkerEvent::Stopped { worker_id, at } => {
            let entry = workers.entry(worker_id.clone()).or_default();
            if entry.state != WorkerState::Error {
                entry.state = WorkerState::Stopped;
            }
            entry.last_heartbeat = Some(at);
            active.remove(&worker_id);
            Some(WorkerEventLog {
                level: "info",
                event: "worker.stopped",
                message: worker_id,
            })
        }
    }
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
