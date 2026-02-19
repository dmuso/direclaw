use super::{append_runtime_log, now_secs, StatePaths, WorkerEvent};
use crate::config::Settings;
use crate::orchestration::function_registry::FunctionRegistry;
use crate::orchestration::routing::process_queued_message_with_runner_binaries;
use crate::orchestration::run_store::{RunState, StepAttemptRecord, WorkflowRunStore};
use crate::orchestration::scheduler::parse_trigger_envelope;
use crate::orchestration::slack_target::{
    parse_slack_target_ref, slack_target_from_conversation, slack_target_ref_to_value,
    validate_profile_mapping, SlackPostingMode,
};
use crate::orchestration::transitions::RoutedSelectorAction;
use crate::provider::RunnerBinaries;
use crate::queue::{self, OutgoingMessage, QueuePaths};
use crate::runtime::recovery::recovered_processing_filename;
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
struct ScopedClaimedMessage {
    queue_paths: QueuePaths,
    claimed: queue::ClaimedMessage,
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
    let queue_sets = collect_orchestrator_queue_paths(settings)?;
    for paths in &queue_sets {
        fs::create_dir_all(&paths.incoming).map_err(|e| e.to_string())?;
        fs::create_dir_all(&paths.processing).map_err(|e| e.to_string())?;
        fs::create_dir_all(&paths.outgoing).map_err(|e| e.to_string())?;
    }
    let mut scheduler = queue::PerKeyScheduler::default();
    let (result_tx, result_rx) = mpsc::channel::<QueueTaskCompletion>();
    let mut in_flight = 0usize;
    let mut processed = 0usize;

    let mut claimed_any = true;
    while claimed_any {
        claimed_any = false;
        for queue_paths in &queue_sets {
            while let Some(claimed) = queue::claim_oldest(queue_paths).map_err(|e| e.to_string())? {
                let key = queue::derive_ordering_key(&claimed.payload);
                scheduler.enqueue(
                    key,
                    ScopedClaimedMessage {
                        queue_paths: queue_paths.clone(),
                        claimed,
                    },
                );
                claimed_any = true;
            }
        }
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
    match recover_processing_queue_entries_for_settings(&state_root, &settings) {
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

    let queue_sets = match collect_orchestrator_queue_paths(&settings) {
        Ok(paths) => paths,
        Err(error) => {
            let _ = events.send(WorkerEvent::Error {
                worker_id: worker_id.clone(),
                at: now_secs(),
                message: error,
                fatal: false,
            });
            return;
        }
    };
    for paths in &queue_sets {
        if let Err(error) = fs::create_dir_all(&paths.incoming)
            .and_then(|_| fs::create_dir_all(&paths.processing))
            .and_then(|_| fs::create_dir_all(&paths.outgoing))
        {
            let _ = events.send(WorkerEvent::Error {
                worker_id: worker_id.clone(),
                at: now_secs(),
                message: error.to_string(),
                fatal: false,
            });
            return;
        }
    }

    let (result_tx, result_rx) = mpsc::channel::<QueueTaskCompletion>();
    let mut scheduler = queue::PerKeyScheduler::default();
    let mut in_flight = 0usize;
    let mut backoff_ms = QUEUE_MIN_POLL_MS;
    loop {
        let stopping = stop.load(Ordering::Relaxed);

        if !stopping {
            let mut claim_budget = config.max_concurrency.saturating_mul(4);
            while claim_budget > 0 {
                let mut claimed_any = false;
                for queue_paths in &queue_sets {
                    match queue::claim_oldest(queue_paths) {
                        Ok(Some(claimed)) => {
                            let key = queue::derive_ordering_key(&claimed.payload);
                            scheduler.enqueue(
                                key,
                                ScopedClaimedMessage {
                                    queue_paths: queue_paths.clone(),
                                    claimed,
                                },
                            );
                            claimed_any = true;
                            claim_budget = claim_budget.saturating_sub(1);
                            if claim_budget == 0 {
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(err) => {
                            let _ = events.send(WorkerEvent::Error {
                                worker_id: worker_id.clone(),
                                at: now_secs(),
                                message: err.to_string(),
                                fatal: false,
                            });
                        }
                    }
                }
                if !claimed_any {
                    break;
                }
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
                    let _ =
                        queue::requeue_failure(&pending.value.queue_paths, &pending.value.claimed);
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
    scheduler: &mut queue::PerKeyScheduler<ScopedClaimedMessage>,
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
    _state_root: &Path,
    settings: &Settings,
    scoped: ScopedClaimedMessage,
    binaries: &RunnerBinaries,
) -> Result<(), String> {
    let run_store = WorkflowRunStore::new(&scoped.queue_paths.root);
    let functions = FunctionRegistry::v1_defaults(run_store.clone(), settings);

    let action = process_queued_message_with_runner_binaries(
        &scoped.queue_paths.root,
        settings,
        &scoped.claimed.payload,
        now_secs(),
        &BTreeMap::new(),
        &functions,
        Some(binaries.clone()),
        |_attempt, _request, _orchestrator_cfg| None,
    )
    .map_err(|e| {
        let _ = queue::requeue_failure(&scoped.queue_paths, &scoped.claimed);
        e.to_string()
    })?;

    let responses = action_to_outbound_messages(&action, &run_store);
    let outbound_slack_target = resolve_outbound_slack_target(settings, &scoped.claimed.payload)?;
    let outbound_channel = outbound_slack_target
        .as_ref()
        .map(|_| "slack".to_string())
        .unwrap_or_else(|| scoped.claimed.payload.channel.clone());
    let outbound_channel_profile_id = outbound_slack_target
        .as_ref()
        .map(|target| target.channel_profile_id.clone())
        .or_else(|| scoped.claimed.payload.channel_profile_id.clone());
    let outbound_conversation_id = outbound_slack_target
        .as_ref()
        .map(|target| match target.posting_mode {
            SlackPostingMode::ChannelPost => target.channel_id.clone(),
            SlackPostingMode::ThreadReply => format!(
                "{}:{}",
                target.channel_id,
                target.thread_ts.clone().unwrap_or_default()
            ),
        })
        .or_else(|| scoped.claimed.payload.conversation_id.clone());
    let outbound_target_ref = outbound_slack_target
        .as_ref()
        .map(slack_target_ref_to_value);
    let now = now_secs();
    let outgoing: Vec<OutgoingMessage> = responses
        .into_iter()
        .enumerate()
        .map(|(index, (message, agent))| OutgoingMessage {
            channel: outbound_channel.clone(),
            channel_profile_id: outbound_channel_profile_id.clone(),
            sender: scoped.claimed.payload.sender.clone(),
            message,
            original_message: scoped.claimed.payload.message.clone(),
            timestamp: now.saturating_add(index as i64),
            message_id: scoped.claimed.payload.message_id.clone(),
            agent,
            conversation_id: outbound_conversation_id.clone(),
            target_ref: outbound_target_ref.clone(),
            files: Vec::new(),
            workflow_run_id: scoped.claimed.payload.workflow_run_id.clone(),
            workflow_step_id: scoped.claimed.payload.workflow_step_id.clone(),
        })
        .collect();
    queue::complete_success_many(&scoped.queue_paths, &scoped.claimed, &outgoing)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn collect_orchestrator_queue_paths(settings: &Settings) -> Result<Vec<QueuePaths>, String> {
    let mut paths = Vec::new();
    for orchestrator_id in settings.orchestrators.keys() {
        let root = settings
            .resolve_orchestrator_runtime_root(orchestrator_id)
            .map_err(|err| err.to_string())?;
        paths.push(QueuePaths::from_state_root(&root));
    }
    Ok(paths)
}

fn recover_processing_queue_entries_for_settings(
    _state_root: &Path,
    settings: &Settings,
) -> Result<Vec<PathBuf>, String> {
    let mut recovered = Vec::new();
    for queue_paths in collect_orchestrator_queue_paths(settings)? {
        recovered.extend(recover_queue_processing_paths(&queue_paths)?);
    }
    Ok(recovered)
}

fn recover_queue_processing_paths(queue_paths: &QueuePaths) -> Result<Vec<PathBuf>, String> {
    let mut recovered = Vec::new();
    let mut entries = Vec::new();
    for entry in fs::read_dir(&queue_paths.processing).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() {
            entries.push(path);
        }
    }
    entries.sort();
    for (index, processing_path) in entries.into_iter().enumerate() {
        let name = processing_path
            .file_name()
            .and_then(|v| v.to_str())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("message.json");
        let target = queue_paths
            .incoming
            .join(recovered_processing_filename(index, name));
        fs::rename(&processing_path, &target).map_err(|e| {
            format!(
                "failed to recover processing file {}: {}",
                processing_path.display(),
                e
            )
        })?;
        recovered.push(target);
    }
    Ok(recovered)
}

fn resolve_outbound_slack_target(
    settings: &Settings,
    inbound: &queue::IncomingMessage,
) -> Result<Option<crate::orchestration::slack_target::SlackTargetRef>, String> {
    if inbound.channel == "slack" {
        let channel_profile_id = inbound
            .channel_profile_id
            .as_deref()
            .ok_or_else(|| "slack message missing channel_profile_id".to_string())?;
        let conversation_id = inbound
            .conversation_id
            .as_deref()
            .ok_or_else(|| "slack message missing conversation_id".to_string())?;
        return slack_target_from_conversation(channel_profile_id, conversation_id).map(Some);
    }

    if inbound.channel == "scheduler" {
        let envelope = parse_trigger_envelope(&inbound.message)?;
        let slack_target = envelope
            .target_ref
            .as_ref()
            .map(|value| parse_slack_target_ref(value, "targetRef"))
            .transpose()?
            .flatten();
        validate_profile_mapping(settings, &envelope.orchestrator_id, slack_target.as_ref())?;
        return Ok(slack_target);
    }

    Ok(None)
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

fn action_to_outbound_messages(
    action: &RoutedSelectorAction,
    run_store: &WorkflowRunStore,
) -> Vec<(String, String)> {
    match action {
        RoutedSelectorAction::WorkflowStart { run_id, .. } => {
            workflow_lifecycle_messages(run_store, run_id)
        }
        RoutedSelectorAction::WorkflowStatus {
            run_id,
            progress,
            message,
        } => {
            if let Some(progress) = progress {
                vec![(
                    format!(
                        "{message}\nrun_id={}\nstate={}",
                        run_id.clone().unwrap_or_else(|| "none".to_string()),
                        progress.state,
                    ),
                    "orchestrator".to_string(),
                )]
            } else {
                vec![(message.clone(), "orchestrator".to_string())]
            }
        }
        RoutedSelectorAction::DiagnosticsInvestigate { findings, .. } => {
            vec![(findings.clone(), "diagnostics".to_string())]
        }
        RoutedSelectorAction::CommandInvoke { result } => {
            let rendered = serde_json::to_string_pretty(result)
                .unwrap_or_else(|_| "command completed".to_string());
            vec![(rendered, "command".to_string())]
        }
    }
}

fn latest_succeeded_attempt(state_root: &Path, run_id: &str) -> Option<StepAttemptRecord> {
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

    latest
}

fn workflow_lifecycle_messages(
    run_store: &WorkflowRunStore,
    run_id: &str,
) -> Vec<(String, String)> {
    let mut messages = Vec::new();
    let attempts = step_attempts_by_time(run_store.state_root(), run_id);
    if attempts.is_empty() {
        messages.push((
            format!("workflow started\nrun_id={run_id}"),
            "orchestrator".to_string(),
        ));
        return messages;
    }

    for attempt in &attempts {
        messages.push((
            format!(
                "Running step `{}` (attempt {})...",
                attempt.step_id, attempt.attempt
            ),
            "orchestrator".to_string(),
        ));
        match attempt.state.as_str() {
            "failed_retryable" => messages.push((
                format!(
                    "Step `{}` failed on attempt {}. Retrying.",
                    attempt.step_id, attempt.attempt
                ),
                "orchestrator".to_string(),
            )),
            "failed" => messages.push((
                format!(
                    "Step `{}` failed on attempt {}: {}",
                    attempt.step_id,
                    attempt.attempt,
                    attempt
                        .error
                        .clone()
                        .filter(|v| !v.trim().is_empty())
                        .unwrap_or_else(|| "unknown error".to_string())
                ),
                "orchestrator".to_string(),
            )),
            _ => {}
        }
    }

    let final_message = final_user_message(run_store, run_id, attempts.last());
    messages.push((final_message, "orchestrator".to_string()));
    messages
}

fn step_attempts_by_time(state_root: &Path, run_id: &str) -> Vec<StepAttemptRecord> {
    let steps_root = state_root.join("workflows/runs").join(run_id).join("steps");
    let mut attempts = Vec::new();
    let Ok(steps) = fs::read_dir(&steps_root) else {
        return attempts;
    };

    for step in steps.flatten() {
        let attempts_root = step.path().join("attempts");
        let Ok(entries) = fs::read_dir(&attempts_root) else {
            continue;
        };
        for entry in entries.flatten() {
            let result_path = entry.path().join("result.json");
            if !result_path.is_file() {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&result_path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<StepAttemptRecord>(&raw) else {
                continue;
            };
            attempts.push(record);
        }
    }

    attempts.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.ended_at.cmp(&b.ended_at))
            .then_with(|| a.step_id.cmp(&b.step_id))
            .then_with(|| a.attempt.cmp(&b.attempt))
    });
    attempts
}

fn final_user_message(
    run_store: &WorkflowRunStore,
    run_id: &str,
    last_attempt: Option<&StepAttemptRecord>,
) -> String {
    let run = run_store.load_run(run_id).ok();
    if let Some(attempt) = last_attempt {
        if attempt.state == "succeeded" {
            return select_final_output_message(
                &attempt.outputs,
                final_output_priority(&attempt.final_output_priority),
            )
            .unwrap_or_else(|| "workflow completed".to_string());
        }
    }

    if let Some(run) = run {
        if run.state == RunState::Canceled {
            return run
                .terminal_reason
                .unwrap_or_else(|| "workflow canceled".to_string());
        }
        if run.state == RunState::Failed {
            return run
                .terminal_reason
                .unwrap_or_else(|| "workflow failed".to_string());
        }
    }

    if let Some(attempt) = latest_succeeded_attempt(run_store.state_root(), run_id) {
        if let Some(message) = select_final_output_message(
            &attempt.outputs,
            final_output_priority(&attempt.final_output_priority),
        ) {
            return message;
        }
        if let Some(summary) = attempt
            .outputs
            .get("summary")
            .and_then(|value| output_value_for_label(value, "summary"))
        {
            return summary;
        }
    }
    run_store
        .load_progress(run_id)
        .ok()
        .map(|progress| progress.summary)
        .filter(|summary| !summary.trim().is_empty())
        .unwrap_or_else(|| "workflow update unavailable".to_string())
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

fn output_value_for_label(value: &Value, label: &str) -> Option<String> {
    output_value_text(value).map(|text| extract_output_label_value(&text, label).unwrap_or(text))
}

fn final_output_priority(configured: &[String]) -> Vec<&str> {
    if configured.is_empty() {
        vec!["artifact", "summary"]
    } else {
        configured.iter().map(String::as_str).collect()
    }
}

fn select_final_output_message(
    outputs: &Map<String, Value>,
    priority: Vec<&str>,
) -> Option<String> {
    for key in priority {
        if let Some(message) = outputs
            .get(key)
            .and_then(|value| output_value_for_label(value, key))
        {
            return Some(message);
        }
    }
    None
}

fn extract_output_label_value(text: &str, label: &str) -> Option<String> {
    const OUTPUT_LABEL_KEYS: [&str; 7] = [
        "status", "summary", "artifact", "decision", "feedback", "plan", "result",
    ];

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lowered = trimmed.to_ascii_lowercase();
    let mut labels: Vec<(usize, usize, &str)> = Vec::new();
    for key in OUTPUT_LABEL_KEYS {
        let token = format!("{key}:");
        let mut from = 0usize;
        while let Some(rel_idx) = lowered[from..].find(&token) {
            let idx = from + rel_idx;
            labels.push((idx, token.len(), key));
            from = idx + token.len();
        }
    }
    labels.sort_by_key(|(idx, _, _)| *idx);

    for (idx, token_len, key) in labels.iter().copied() {
        if key != label {
            continue;
        }
        let start = idx + token_len;
        let end = labels
            .iter()
            .find_map(|(next_idx, _, _)| (*next_idx > idx).then_some(*next_idx))
            .unwrap_or(trimmed.len());
        if start >= end || end > trimmed.len() {
            continue;
        }
        let candidate = trimmed[start..end].trim();
        if !candidate.is_empty() {
            return Some(candidate.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::final_user_message;
    use crate::orchestration::run_store::{StepAttemptRecord, WorkflowRunStore};
    use serde_json::{Map, Value};
    use tempfile::tempdir;

    fn succeeded_attempt_with_outputs(
        summary: &str,
        artifact: &str,
        final_output_priority: &[&str],
    ) -> StepAttemptRecord {
        let mut outputs = Map::new();
        outputs.insert("summary".to_string(), Value::String(summary.to_string()));
        outputs.insert("artifact".to_string(), Value::String(artifact.to_string()));
        StepAttemptRecord {
            run_id: "run-1".to_string(),
            step_id: "answer".to_string(),
            attempt: 1,
            started_at: 10,
            ended_at: 20,
            state: "succeeded".to_string(),
            outputs,
            output_files: Default::default(),
            final_output_priority: final_output_priority
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            next_step_id: None,
            error: None,
            output_validation_errors: Default::default(),
        }
    }

    #[test]
    fn final_message_prefers_artifact_for_quick_answer_workflow() {
        let dir = tempdir().expect("tempdir");
        let store = WorkflowRunStore::new(dir.path());
        store
            .create_run("run-1", "quick_answer", 1)
            .expect("create run");
        let attempt = succeeded_attempt_with_outputs(
            "summary output",
            "artifact output",
            &["artifact", "summary"],
        );

        let message = final_user_message(&store, "run-1", Some(&attempt));
        assert_eq!(message, "artifact output");
    }

    #[test]
    fn final_message_prefers_artifact_for_non_quick_answer_workflow() {
        let dir = tempdir().expect("tempdir");
        let store = WorkflowRunStore::new(dir.path());
        store.create_run("run-1", "plan", 1).expect("create run");
        let attempt = succeeded_attempt_with_outputs(
            "summary output",
            "artifact output",
            &["artifact", "summary"],
        );

        let message = final_user_message(&store, "run-1", Some(&attempt));
        assert_eq!(message, "artifact output");
    }

    #[test]
    fn final_message_uses_artifact_value_without_label_for_quick_answer() {
        let dir = tempdir().expect("tempdir");
        let store = WorkflowRunStore::new(dir.path());
        store
            .create_run("run-1", "quick_answer", 1)
            .expect("create run");
        let attempt = succeeded_attempt_with_outputs(
            "summary output",
            "status: complete\nsummary: concise summary\nartifact: Hello! I am ready.",
            &["artifact", "summary"],
        );

        let message = final_user_message(&store, "run-1", Some(&attempt));
        assert_eq!(message, "Hello! I am ready.");
    }

    #[test]
    fn final_message_uses_summary_value_without_label_for_non_quick_answer() {
        let dir = tempdir().expect("tempdir");
        let store = WorkflowRunStore::new(dir.path());
        store.create_run("run-1", "plan", 1).expect("create run");
        let attempt = succeeded_attempt_with_outputs(
            "status: complete\nsummary: concise summary\nartifact: Hello! I am ready.",
            "artifact output",
            &["summary", "artifact"],
        );

        let message = final_user_message(&store, "run-1", Some(&attempt));
        assert_eq!(message, "concise summary");
    }

    #[test]
    fn final_message_defaults_to_artifact_then_summary_when_priority_missing() {
        let dir = tempdir().expect("tempdir");
        let store = WorkflowRunStore::new(dir.path());
        store.create_run("run-1", "plan", 1).expect("create run");
        let attempt = succeeded_attempt_with_outputs("summary output", "artifact output", &[]);

        let message = final_user_message(&store, "run-1", Some(&attempt));
        assert_eq!(message, "artifact output");
    }
}
