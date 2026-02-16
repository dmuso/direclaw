use super::{
    append_runtime_log, now_secs, recover_processing_queue_entries, StatePaths, WorkerEvent,
};
use crate::config::Settings;
use crate::orchestration::function_registry::FunctionRegistry;
use crate::orchestration::routing::process_queued_message_with_runner_binaries;
use crate::orchestration::run_store::{StepAttemptRecord, WorkflowRunStore};
use crate::orchestration::selector::run_selector_attempt_with_provider;
use crate::orchestration::transitions::RoutedSelectorAction;
use crate::provider::RunnerBinaries;
use crate::queue::{self, OutgoingMessage, QueuePaths};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub const QUEUE_MAX_CONCURRENCY: usize = 4;
pub const QUEUE_MIN_POLL_MS: u64 = 100;
pub const QUEUE_MAX_POLL_MS: u64 = 1000;

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
                    thread::sleep(slow_shutdown_delay());
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
                    thread::sleep(slow_shutdown_delay());
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
    let functions = FunctionRegistry::v1_defaults(run_store.clone(), settings);

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

    let (response_message, response_agent) =
        action_to_outbound(&action, &claimed.payload.channel, &run_store);
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

fn sleep_with_stop(stop: &AtomicBool, total: Duration) -> bool {
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

fn slow_shutdown_delay() -> Duration {
    let seconds = std::env::var("DIRECLAW_SLOW_SHUTDOWN_DELAY_SECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(6);
    Duration::from_secs(seconds)
}

fn action_to_outbound(
    action: &RoutedSelectorAction,
    inbound_channel: &str,
    run_store: &WorkflowRunStore,
) -> (String, String) {
    match action {
        RoutedSelectorAction::WorkflowStart {
            run_id,
            workflow_id,
        } => {
            if inbound_channel == "local" {
                if let Some(message) = local_workflow_answer(run_store, run_id) {
                    return (message, "orchestrator".to_string());
                }
            }
            (
                format!("workflow started\nrun_id={run_id}\nworkflow_id={workflow_id}"),
                "orchestrator".to_string(),
            )
        }
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

fn local_workflow_answer(run_store: &WorkflowRunStore, run_id: &str) -> Option<String> {
    let run = run_store.load_run(run_id).ok()?;
    if !run.state.is_terminal() {
        return None;
    }
    if let Some(outputs) = latest_succeeded_attempt_outputs(run_store.state_root(), run_id) {
        if let Some(answer) = select_user_visible_output(&outputs) {
            return Some(answer);
        }
    }
    run_store
        .load_progress(run_id)
        .ok()
        .map(|progress| progress.summary)
        .filter(|summary| !summary.trim().is_empty())
}

fn latest_succeeded_attempt_outputs(state_root: &Path, run_id: &str) -> Option<Map<String, Value>> {
    let steps_root = state_root.join("workflows/runs").join(run_id).join("steps");
    let mut latest: Option<StepAttemptRecord> = None;

    for step in fs::read_dir(&steps_root).ok()?.flatten() {
        let attempts_root = step.path().join("attempts");
        let Ok(attempts) = fs::read_dir(&attempts_root) else {
            continue;
        };
        for attempt in attempts.flatten() {
            let result_path = attempt.path().join("result.json");
            if !result_path.is_file() {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&result_path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<StepAttemptRecord>(&raw) else {
                continue;
            };
            if record.state != "succeeded" {
                continue;
            }
            let replace = latest
                .as_ref()
                .map(|current| record.ended_at > current.ended_at)
                .unwrap_or(true);
            if replace {
                latest = Some(record);
            }
        }
    }

    latest.map(|record| record.outputs)
}

fn select_user_visible_output(outputs: &Map<String, Value>) -> Option<String> {
    for key in ["answer", "response", "summary", "result", "artifact"] {
        if let Some(text) = outputs.get(key).and_then(output_value_text) {
            return Some(text);
        }
    }
    outputs.values().find_map(output_value_text)
}

fn output_value_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        other => serde_json::to_string(other)
            .ok()
            .filter(|text| !text.trim().is_empty()),
    }
}
