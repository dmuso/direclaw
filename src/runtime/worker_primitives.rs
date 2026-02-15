use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

pub(crate) const QUEUE_MAX_CONCURRENCY: usize = 4;
pub(crate) const QUEUE_MIN_POLL_MS: u64 = 100;
pub(crate) const QUEUE_MAX_POLL_MS: u64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuePollingDefaults {
    pub max_concurrency: usize,
    pub min_poll_ms: u64,
    pub max_poll_ms: u64,
}

pub fn queue_polling_defaults() -> QueuePollingDefaults {
    QueuePollingDefaults {
        max_concurrency: QUEUE_MAX_CONCURRENCY,
        min_poll_ms: QUEUE_MIN_POLL_MS,
        max_poll_ms: QUEUE_MAX_POLL_MS,
    }
}

#[derive(Debug, Clone)]
pub(crate) enum WorkerEvent {
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

pub(crate) fn sleep_with_stop(stop: &AtomicBool, total: Duration) -> bool {
    let mut remaining = total;
    while remaining > Duration::from_millis(0) {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let step = remaining.min(Duration::from_millis(200));
        thread::sleep(step);
        remaining = remaining.saturating_sub(step);
    }
    !stop.load(Ordering::Relaxed)
}
