use super::{heartbeat_worker, now_secs, queue_worker, sleep_with_stop, WorkerEvent};
use crate::channels::slack;
use crate::config::Settings;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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

#[derive(Debug, Clone)]
pub(crate) enum WorkerRuntime {
    QueueProcessor,
    OrchestratorDispatcher,
    Slack,
    Heartbeat,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerSpec {
    pub(crate) id: String,
    pub(crate) runtime: WorkerRuntime,
    pub(crate) interval: Duration,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerRunContext {
    pub(crate) state_root: PathBuf,
    pub(crate) settings: Settings,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) events: Sender<WorkerEvent>,
    pub(crate) should_fail: bool,
    pub(crate) slow_shutdown: bool,
    pub(crate) queue_max_concurrency: usize,
}

pub fn tick_slack_worker(state_root: &Path, settings: &Settings) -> Result<(), String> {
    slack::sync_once(state_root, settings)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub(crate) fn build_worker_specs(settings: &Settings) -> Vec<WorkerSpec> {
    let mut specs = Vec::new();
    specs.push(WorkerSpec {
        id: "queue_processor".to_string(),
        runtime: WorkerRuntime::QueueProcessor,
        interval: Duration::from_millis(250),
    });
    specs.push(WorkerSpec {
        id: "orchestrator_dispatcher".to_string(),
        runtime: WorkerRuntime::OrchestratorDispatcher,
        interval: Duration::from_secs(1),
    });

    if settings.monitoring.heartbeat_interval.unwrap_or(0) > 0 {
        specs.push(WorkerSpec {
            id: "heartbeat".to_string(),
            runtime: WorkerRuntime::Heartbeat,
            interval: Duration::from_secs(settings.monitoring.heartbeat_interval.unwrap_or(3600)),
        });
    }

    for (channel, config) in &settings.channels {
        if !config.enabled {
            continue;
        }
        if channel == "slack" {
            specs.push(WorkerSpec {
                id: format!("channel:{channel}"),
                runtime: WorkerRuntime::Slack,
                interval: Duration::from_secs(2),
            });
        }
    }

    specs
}

pub(crate) fn run_worker(spec: WorkerSpec, context: WorkerRunContext) {
    let WorkerRunContext {
        state_root,
        settings,
        stop,
        events,
        should_fail,
        slow_shutdown,
        queue_max_concurrency,
    } = context;

    let _ = events.send(WorkerEvent::Started {
        worker_id: spec.id.clone(),
        at: now_secs(),
    });

    if matches!(spec.runtime, WorkerRuntime::Slack) {
        if let Err(err) = slack::validate_startup_credentials(&settings) {
            let _ = events.send(WorkerEvent::Error {
                worker_id: spec.id.clone(),
                at: now_secs(),
                message: err.to_string(),
                fatal: true,
            });
            let _ = events.send(WorkerEvent::Stopped {
                worker_id: spec.id,
                at: now_secs(),
            });
            return;
        }
    }

    if should_fail {
        let _ = events.send(WorkerEvent::Error {
            worker_id: spec.id.clone(),
            at: now_secs(),
            message: "fault injection requested".to_string(),
            fatal: true,
        });
        let _ = events.send(WorkerEvent::Stopped {
            worker_id: spec.id,
            at: now_secs(),
        });
        return;
    }

    if matches!(spec.runtime, WorkerRuntime::QueueProcessor) {
        queue_worker::run_queue_processor_loop(
            spec.id,
            state_root,
            settings,
            stop,
            events,
            slow_shutdown,
            queue_max_concurrency,
        );
        return;
    }

    loop {
        if stop.load(Ordering::Relaxed) {
            if slow_shutdown {
                thread::sleep(Duration::from_secs(6));
            }
            break;
        }

        let tick = match spec.runtime {
            WorkerRuntime::QueueProcessor => Ok(()),
            WorkerRuntime::OrchestratorDispatcher => Ok(()),
            WorkerRuntime::Slack => tick_slack_worker(&state_root, &settings),
            WorkerRuntime::Heartbeat => heartbeat_worker::tick_heartbeat_worker(),
        };

        match tick {
            Ok(()) => {
                let _ = events.send(WorkerEvent::Heartbeat {
                    worker_id: spec.id.clone(),
                    at: now_secs(),
                });
            }
            Err(message) => {
                let _ = events.send(WorkerEvent::Error {
                    worker_id: spec.id.clone(),
                    at: now_secs(),
                    message,
                    fatal: false,
                });
            }
        }

        if !sleep_with_stop(&stop, spec.interval) {
            if slow_shutdown {
                thread::sleep(Duration::from_secs(6));
            }
            break;
        }
    }

    let _ = events.send(WorkerEvent::Stopped {
        worker_id: spec.id,
        at: now_secs(),
    });
}
