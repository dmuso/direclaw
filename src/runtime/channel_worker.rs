use super::{
    heartbeat_worker, memory_worker, now_secs, queue_worker, scheduler_worker, WorkerEvent,
};
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
    Memory,
    Scheduler,
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
    match slack::sync_runtime_once(state_root, settings) {
        Ok(_) => Ok(()),
        Err(slack::SlackError::RateLimited {
            retry_after_secs, ..
        }) => {
            thread::sleep(rate_limit_sleep_duration(retry_after_secs));
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn rate_limit_sleep_duration(retry_after_secs: u64) -> Duration {
    let requested = Duration::from_secs(retry_after_secs);
    let Some(cap_ms) = std::env::var("DIRECLAW_SLACK_RATE_LIMIT_SLEEP_MAX_MILLISECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
    else {
        return requested;
    };
    requested.min(Duration::from_millis(cap_ms))
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
    if settings.memory.enabled {
        specs.push(WorkerSpec {
            id: "memory_worker".to_string(),
            runtime: WorkerRuntime::Memory,
            interval: Duration::from_secs(settings.memory.worker_interval_seconds),
        });
    }
    specs.push(WorkerSpec {
        id: "scheduler".to_string(),
        runtime: WorkerRuntime::Scheduler,
        interval: Duration::from_secs(60),
    });

    if let Some(interval) = heartbeat_worker::configured_heartbeat_interval(settings) {
        specs.push(WorkerSpec {
            id: "heartbeat".to_string(),
            runtime: WorkerRuntime::Heartbeat,
            interval,
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

    if matches!(spec.runtime, WorkerRuntime::Memory) {
        if let Err(err) = memory_worker::bootstrap_memory_runtime_paths(&settings) {
            let _ = events.send(WorkerEvent::Error {
                worker_id: spec.id.clone(),
                at: now_secs(),
                message: err,
                fatal: true,
            });
            let _ = events.send(WorkerEvent::Stopped {
                worker_id: spec.id,
                at: now_secs(),
            });
            return;
        }
    }

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
                thread::sleep(slow_shutdown_delay());
            }
            break;
        }

        let tick = match spec.runtime {
            WorkerRuntime::QueueProcessor => Ok(()),
            WorkerRuntime::OrchestratorDispatcher => Ok(()),
            WorkerRuntime::Memory => memory_worker::tick_memory_worker(&settings),
            WorkerRuntime::Scheduler => {
                scheduler_worker::tick_scheduler_worker(&state_root, &settings)
            }
            WorkerRuntime::Slack => tick_slack_worker(&state_root, &settings),
            WorkerRuntime::Heartbeat => {
                heartbeat_worker::tick_heartbeat_worker(&state_root, &settings)
            }
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
                thread::sleep(slow_shutdown_delay());
            }
            break;
        }
    }

    let _ = events.send(WorkerEvent::Stopped {
        worker_id: spec.id,
        at: now_secs(),
    });
}

fn sleep_with_stop(stop: &AtomicBool, total: Duration) -> bool {
    let mut remaining = total;
    while remaining > Duration::from_millis(0) {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let step = remaining.min(Duration::from_millis(25));
        thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
    !stop.load(Ordering::Relaxed)
}

fn slow_shutdown_delay() -> Duration {
    if let Some(milliseconds) = std::env::var("DIRECLAW_SLOW_SHUTDOWN_DELAY_MILLISECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
    {
        return Duration::from_millis(milliseconds);
    }
    let seconds = std::env::var("DIRECLAW_SLOW_SHUTDOWN_DELAY_SECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(6);
    Duration::from_secs(seconds)
}
