use super::{
    append_runtime_log, now_secs, recover_processing_queue_entries, sleep_with_stop, StatePaths,
    WorkerEvent, QUEUE_MAX_POLL_MS, QUEUE_MIN_POLL_MS,
};
use crate::config::Settings;
use crate::orchestration::function_registry::FunctionRegistry;
use crate::orchestration::routing::process_queued_message_with_runner_binaries;
use crate::orchestration::run_store::WorkflowRunStore;
use crate::orchestration::selector::run_selector_attempt_with_provider;
use crate::orchestration::transitions::RoutedSelectorAction;
use crate::provider::RunnerBinaries;
use crate::queue::{self, OutgoingMessage, QueuePaths};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug)]
struct QueueTaskCompletion {
    key: queue::OrderingKey,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct QueueProcessorLoopConfig {
    pub(crate) slow_shutdown: bool,
    pub(crate) max_concurrency: usize,
    pub(crate) binaries: RunnerBinaries,
}

pub fn drain_queue_once(
    state_root: &Path,
    settings: &Settings,
    max_concurrency: usize,
) -> Result<usize, String> {
    let binaries = resolve_runner_binaries();
    drain_queue_once_with_binaries(state_root, settings, max_concurrency, &binaries)
}

pub fn drain_queue_once_with_binaries(
    state_root: &Path,
    settings: &Settings,
    max_concurrency: usize,
    binaries: &RunnerBinaries,
) -> Result<usize, String> {
    let queue_paths = QueuePaths::from_state_root(state_root);
    let mut scheduler = queue::PerKeyScheduler::default();
    let (result_tx, result_rx) = mpsc::channel::<QueueTaskCompletion>();
    let mut in_flight = 0usize;
    let mut processed = 0usize;

    while let Some(claimed) = queue::claim_oldest(&queue_paths).map_err(|e| e.to_string())? {
        let key = queue::derive_ordering_key(&claimed.payload);
        scheduler.enqueue(key, claimed);
    }

    loop {
        let available = max_concurrency.saturating_sub(in_flight);
        if available > 0 {
            for scheduled in scheduler.dequeue_runnable(available) {
                let tx = result_tx.clone();
                let root = state_root.to_path_buf();
                let cfg = settings.clone();
                let bins = binaries.clone();
                let _ = thread::spawn(move || {
                    let result = process_claimed_message(&root, &cfg, scheduled.value, &bins).err();
                    let _ = tx.send(QueueTaskCompletion {
                        key: scheduled.key,
                        error: result,
                    });
                });
                in_flight += 1;
            }
        }

        if in_flight == 0 {
            break;
        }

        let completion = match result_rx.recv_timeout(Duration::from_millis(QUEUE_MIN_POLL_MS)) {
            Ok(done) => done,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err("queue worker completion channel disconnected".to_string())
            }
        };
        in_flight = in_flight.saturating_sub(1);
        scheduler.complete(&completion.key);
        if completion.error.is_none() {
            processed += 1;
        } else if let Some(error) = completion.error {
            return Err(error);
        }
    }

    Ok(processed)
}

pub(crate) fn run_queue_processor_loop(
    worker_id: String,
    state_root: PathBuf,
    settings: Settings,
    stop: Arc<AtomicBool>,
    events: Sender<WorkerEvent>,
    slow_shutdown: bool,
    max_concurrency: usize,
) {
    let config = QueueProcessorLoopConfig {
        slow_shutdown,
        max_concurrency,
        binaries: resolve_runner_binaries(),
    };
    run_queue_processor_loop_with_binaries(worker_id, state_root, settings, stop, events, config);
}

pub(crate) fn run_queue_processor_loop_with_binaries(
    worker_id: String,
    state_root: PathBuf,
    settings: Settings,
    stop: Arc<AtomicBool>,
    events: Sender<WorkerEvent>,
    config: QueueProcessorLoopConfig,
) {
    match recover_processing_queue_entries(&state_root) {
        Ok(recovered) => {
            for path in recovered {
                append_runtime_log(
                    &StatePaths::new(&state_root),
                    "info",
                    "queue.recovered",
                    &format!("requeued {}", path.display()),
                );
            }
        }
        Err(error) => {
            let _ = events.send(WorkerEvent::Error {
                worker_id: worker_id.clone(),
                at: now_secs(),
                message: error,
                fatal: false,
            });
        }
    }

    let queue_paths = QueuePaths::from_state_root(&state_root);
    let (result_tx, result_rx) = mpsc::channel::<QueueTaskCompletion>();
    let mut scheduler = queue::PerKeyScheduler::default();
    let mut in_flight = 0usize;
    let mut backoff_ms = QUEUE_MIN_POLL_MS;
    loop {
        let stopping = stop.load(Ordering::Relaxed);

        if !stopping {
            let mut claim_budget = config.max_concurrency.saturating_mul(4);
            while claim_budget > 0 {
                match queue::claim_oldest(&queue_paths) {
                    Ok(Some(claimed)) => {
                        let key = queue::derive_ordering_key(&claimed.payload);
                        scheduler.enqueue(key, claimed);
                    }
                    Ok(None) => break,
                    Err(err) => {
                        let _ = events.send(WorkerEvent::Error {
                            worker_id: worker_id.clone(),
                            at: now_secs(),
                            message: err.to_string(),
                            fatal: false,
                        });
                        break;
                    }
                }
                claim_budget -= 1;
            }
        }

        let available_slots = config.max_concurrency.saturating_sub(in_flight);
        if !stopping && available_slots > 0 {
            for scheduled in scheduler.dequeue_runnable(available_slots) {
                let tx = result_tx.clone();
                let root = state_root.clone();
                let cfg = settings.clone();
                let bins = config.binaries.clone();
                let _ = thread::spawn(move || {
                    let err = process_claimed_message(&root, &cfg, scheduled.value, &bins).err();
                    let _ = tx.send(QueueTaskCompletion {
                        key: scheduled.key,
                        error: err,
                    });
                });
                in_flight += 1;
            }
        }

        while let Ok(done) = result_rx.try_recv() {
            handle_queue_task_completion(&worker_id, &events, &mut scheduler, &mut in_flight, done);
        }

        if stopping {
            if in_flight == 0 {
                for pending in scheduler.drain_pending() {
                    let _ = queue::requeue_failure(&queue_paths, &pending.value);
                }
                if config.slow_shutdown {
                    thread::sleep(Duration::from_secs(6));
                }
                break;
            }
            match result_rx.recv_timeout(Duration::from_millis(QUEUE_MIN_POLL_MS)) {
                Ok(done) => handle_queue_task_completion(
                    &worker_id,
                    &events,
                    &mut scheduler,
                    &mut in_flight,
                    done,
                ),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    in_flight = 0;
                }
            }
            continue;
        }

        if scheduler.pending_len() == 0 && in_flight == 0 {
            let _ = events.send(WorkerEvent::Heartbeat {
                worker_id: worker_id.clone(),
                at: now_secs(),
            });
            if !sleep_with_stop(&stop, Duration::from_millis(backoff_ms)) {
                if config.slow_shutdown {
                    thread::sleep(Duration::from_secs(6));
                }
                break;
            }
            backoff_ms = (backoff_ms.saturating_mul(2)).min(QUEUE_MAX_POLL_MS);
        } else {
            backoff_ms = QUEUE_MIN_POLL_MS;
            thread::sleep(Duration::from_millis(QUEUE_MIN_POLL_MS));
        }
    }

    let _ = events.send(WorkerEvent::Stopped {
        worker_id,
        at: now_secs(),
    });
}

fn handle_queue_task_completion(
    worker_id: &str,
    events: &Sender<WorkerEvent>,
    scheduler: &mut queue::PerKeyScheduler<queue::ClaimedMessage>,
    in_flight: &mut usize,
    done: QueueTaskCompletion,
) {
    *in_flight = in_flight.saturating_sub(1);
    scheduler.complete(&done.key);
    if let Some(message) = done.error {
        let _ = events.send(WorkerEvent::Error {
            worker_id: worker_id.to_string(),
            at: now_secs(),
            message,
            fatal: false,
        });
    } else {
        let _ = events.send(WorkerEvent::Heartbeat {
            worker_id: worker_id.to_string(),
            at: now_secs(),
        });
    }
}

fn process_claimed_message(
    state_root: &Path,
    settings: &Settings,
    claimed: queue::ClaimedMessage,
    binaries: &RunnerBinaries,
) -> Result<(), String> {
    let queue_paths = QueuePaths::from_state_root(state_root);
    let run_store = WorkflowRunStore::new(state_root);
    let functions = FunctionRegistry::v1_defaults(run_store, settings);

    let action = process_queued_message_with_runner_binaries(
        state_root,
        settings,
        &claimed.payload,
        now_secs(),
        &BTreeMap::new(),
        &functions,
        Some(binaries.clone()),
        |attempt, request, orchestrator_cfg| {
            run_selector_attempt_with_provider(
                state_root,
                settings,
                request,
                orchestrator_cfg,
                attempt,
                binaries,
            )
            .ok()
        },
    )
    .map_err(|e| {
        let _ = queue::requeue_failure(&queue_paths, &claimed);
        e.to_string()
    })?;

    let (response_message, response_agent) = action_to_outbound(&action);
    let outgoing = OutgoingMessage {
        channel: claimed.payload.channel.clone(),
        channel_profile_id: claimed.payload.channel_profile_id.clone(),
        sender: claimed.payload.sender.clone(),
        message: response_message,
        original_message: claimed.payload.message.clone(),
        timestamp: now_secs(),
        message_id: claimed.payload.message_id.clone(),
        agent: response_agent,
        conversation_id: claimed.payload.conversation_id.clone(),
        files: Vec::new(),
        workflow_run_id: claimed.payload.workflow_run_id.clone(),
        workflow_step_id: claimed.payload.workflow_step_id.clone(),
    };
    queue::complete_success(&queue_paths, &claimed, &outgoing).map_err(|e| e.to_string())?;
    Ok(())
}

fn resolve_runner_binaries() -> RunnerBinaries {
    RunnerBinaries {
        anthropic: std::env::var("DIRECLAW_PROVIDER_BIN_ANTHROPIC")
            .unwrap_or_else(|_| "claude".to_string()),
        openai: std::env::var("DIRECLAW_PROVIDER_BIN_OPENAI")
            .unwrap_or_else(|_| "codex".to_string()),
    }
}

fn action_to_outbound(action: &RoutedSelectorAction) -> (String, String) {
    match action {
        RoutedSelectorAction::WorkflowStart {
            run_id,
            workflow_id,
        } => (
            format!("workflow started\nrun_id={run_id}\nworkflow_id={workflow_id}"),
            "orchestrator".to_string(),
        ),
        RoutedSelectorAction::WorkflowStatus {
            run_id,
            progress,
            message,
        } => {
            if let Some(progress) = progress {
                (
                    format!(
                        "{message}\nrun_id={}\nstate={}",
                        run_id.clone().unwrap_or_else(|| "none".to_string()),
                        progress.state,
                    ),
                    "orchestrator".to_string(),
                )
            } else {
                (message.clone(), "orchestrator".to_string())
            }
        }
        RoutedSelectorAction::DiagnosticsInvestigate { findings, .. } => {
            (findings.clone(), "diagnostics".to_string())
        }
        RoutedSelectorAction::CommandInvoke { result } => {
            let rendered = serde_json::to_string_pretty(result)
                .unwrap_or_else(|_| "command completed".to_string());
            (rendered, "command".to_string())
        }
    }
}
