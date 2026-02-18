use crate::queue::{IncomingMessage, QueuePaths};
use crate::shared::logging::append_orchestrator_log_line;
use chrono::{Datelike, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CRON_SEARCH_MINUTES: i64 = 60 * 24 * 366 * 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Enabled,
    Paused,
    Disabled,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
    FireOnceOnRecovery,
    SkipMissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScheduleConfig {
    Once {
        run_at: i64,
    },
    Interval {
        every_seconds: u64,
        anchor_at: Option<i64>,
    },
    Cron {
        expression: String,
        timezone: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TargetAction {
    WorkflowStart {
        workflow_id: String,
        #[serde(default)]
        inputs: Map<String, Value>,
    },
    CommandInvoke {
        function_id: String,
        #[serde(default)]
        function_args: Map<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledJob {
    pub job_id: String,
    pub orchestrator_id: String,
    #[serde(default)]
    pub created_by: Map<String, Value>,
    pub schedule: ScheduleConfig,
    pub target_action: TargetAction,
    #[serde(default)]
    pub target_ref: Option<Value>,
    pub state: JobState,
    pub misfire_policy: MisfirePolicy,
    pub next_run_at: Option<i64>,
    #[serde(default)]
    pub last_run_at: Option<i64>,
    #[serde(default)]
    pub last_result: Option<String>,
    pub allow_overlap: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub orchestrator_id: String,
    pub created_by: Map<String, Value>,
    pub schedule: ScheduleConfig,
    pub target_action: TargetAction,
    pub target_ref: Option<Value>,
    pub misfire_policy: MisfirePolicy,
    pub allow_overlap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobPatch {
    pub schedule: Option<ScheduleConfig>,
    pub target_action: Option<TargetAction>,
    pub target_ref: Option<Option<Value>>,
    pub misfire_policy: Option<MisfirePolicy>,
    pub allow_overlap: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTriggerEnvelope {
    pub job_id: String,
    pub execution_id: String,
    pub triggered_at: i64,
    pub orchestrator_id: String,
    pub target_action: TargetAction,
    #[serde(default)]
    pub target_ref: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RunStatus {
    Dispatched,
    SkippedMissed,
    SkippedDuplicate,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveExecution {
    job_id: String,
    execution_id: String,
    started_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunRecord {
    job_id: String,
    execution_id: String,
    triggered_at: i64,
    status: RunStatus,
    created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SchedulerState {
    #[serde(default)]
    last_tick_at: Option<i64>,
    #[serde(default)]
    recent_execution_ids: Vec<String>,
    #[serde(default)]
    active_executions: Vec<ActiveExecution>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchedRun {
    pub job_id: String,
    pub execution_id: String,
    pub triggered_at: i64,
}

#[derive(Debug, Clone)]
pub struct JobStore {
    runtime_root: PathBuf,
}

impl JobStore {
    pub fn new(runtime_root: impl AsRef<Path>) -> Self {
        Self {
            runtime_root: runtime_root.as_ref().to_path_buf(),
        }
    }

    pub fn create(&self, input: NewJob, now: i64) -> Result<ScheduledJob, String> {
        validate_schedule(&input.schedule)?;
        validate_target_action(&input.target_action)?;
        if input.orchestrator_id.trim().is_empty() {
            return Err("orchestrator_id must be non-empty".to_string());
        }

        let job_id = format!(
            "job-{}-{}",
            sanitize_id(&input.orchestrator_id),
            now_nanos().abs()
        );
        let next_run_at = compute_next_run_at(&input.schedule, now, None)?;
        let job = ScheduledJob {
            job_id,
            orchestrator_id: input.orchestrator_id,
            created_by: input.created_by,
            schedule: input.schedule,
            target_action: input.target_action,
            target_ref: input.target_ref,
            state: JobState::Enabled,
            misfire_policy: input.misfire_policy,
            next_run_at,
            last_run_at: None,
            last_result: None,
            allow_overlap: input.allow_overlap,
            created_at: now,
            updated_at: now,
        };
        self.persist_job(&job)?;
        Ok(job)
    }

    pub fn load(&self, job_id: &str) -> Result<ScheduledJob, String> {
        let path = self.job_path(job_id);
        let raw = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|err| format!("failed to parse {}: {err}", path.display()))
    }

    pub fn list_all(&self) -> Result<Vec<ScheduledJob>, String> {
        let mut jobs = Vec::new();
        let dir = self.jobs_dir();
        if !dir.exists() {
            return Ok(jobs);
        }
        for entry in
            fs::read_dir(&dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?
        {
            let path = entry.map_err(|err| err.to_string())?.path();
            if path.extension().and_then(|v| v.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read_to_string(&path)
                .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
            let job: ScheduledJob = serde_json::from_str(&raw)
                .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
            jobs.push(job);
        }
        jobs.sort_by(|a, b| a.job_id.cmp(&b.job_id));
        Ok(jobs)
    }

    pub fn list_for_orchestrator(
        &self,
        orchestrator_id: &str,
    ) -> Result<Vec<ScheduledJob>, String> {
        Ok(self
            .list_all()?
            .into_iter()
            .filter(|job| job.orchestrator_id == orchestrator_id)
            .collect())
    }

    pub fn update(&self, job_id: &str, patch: JobPatch, now: i64) -> Result<ScheduledJob, String> {
        let mut job = self.load(job_id)?;
        if job.state == JobState::Deleted {
            return Err(format!("job `{job_id}` is deleted"));
        }

        if let Some(schedule) = patch.schedule {
            validate_schedule(&schedule)?;
            job.schedule = schedule;
            job.next_run_at = compute_next_run_at(&job.schedule, now, job.last_run_at)?;
        }
        if let Some(action) = patch.target_action {
            validate_target_action(&action)?;
            job.target_action = action;
        }
        if let Some(target_ref) = patch.target_ref {
            job.target_ref = target_ref;
        }
        if let Some(policy) = patch.misfire_policy {
            job.misfire_policy = policy;
        }
        if let Some(allow_overlap) = patch.allow_overlap {
            job.allow_overlap = allow_overlap;
        }

        job.updated_at = now;
        self.persist_job(&job)?;
        Ok(job)
    }

    pub fn pause(&self, job_id: &str, now: i64) -> Result<ScheduledJob, String> {
        self.transition_state(job_id, JobState::Paused, now)
    }

    pub fn resume(&self, job_id: &str, now: i64) -> Result<ScheduledJob, String> {
        self.transition_state(job_id, JobState::Enabled, now)
    }

    pub fn delete(&self, job_id: &str, now: i64) -> Result<ScheduledJob, String> {
        self.transition_state(job_id, JobState::Deleted, now)
    }

    pub fn run_now(&self, job_id: &str, now: i64) -> Result<ScheduledJob, String> {
        let mut job = self.load(job_id)?;
        if job.state != JobState::Enabled {
            return Err(format!(
                "job `{job_id}` must be enabled before run_now (state={})",
                state_name(job.state)
            ));
        }
        job.next_run_at = Some(now);
        job.updated_at = now;
        self.persist_job(&job)?;
        Ok(job)
    }

    fn transition_state(
        &self,
        job_id: &str,
        to: JobState,
        now: i64,
    ) -> Result<ScheduledJob, String> {
        let mut job = self.load(job_id)?;
        if !valid_transition(job.state, to) {
            return Err(format!(
                "invalid scheduler job transition `{}` -> `{}`",
                state_name(job.state),
                state_name(to)
            ));
        }
        job.state = to;
        job.updated_at = now;
        self.persist_job(&job)?;
        Ok(job)
    }

    fn persist_job(&self, job: &ScheduledJob) -> Result<(), String> {
        fs::create_dir_all(self.jobs_dir())
            .map_err(|err| format!("failed to create jobs dir: {err}"))?;
        let path = self.job_path(&job.job_id);
        let body = serde_json::to_vec_pretty(job)
            .map_err(|err| format!("failed to encode job `{}`: {err}", job.job_id))?;
        fs::write(&path, body).map_err(|err| format!("failed to write {}: {err}", path.display()))
    }

    fn jobs_dir(&self) -> PathBuf {
        self.runtime_root.join("automation/jobs")
    }

    fn job_path(&self, job_id: &str) -> PathBuf {
        self.jobs_dir().join(format!("{job_id}.json"))
    }
}

#[derive(Debug, Clone)]
pub struct SchedulerWorker {
    runtime_root: PathBuf,
    store: JobStore,
}

impl SchedulerWorker {
    pub fn new(runtime_root: impl AsRef<Path>) -> Self {
        let runtime_root = runtime_root.as_ref().to_path_buf();
        Self {
            store: JobStore::new(&runtime_root),
            runtime_root,
        }
    }

    pub fn tick(&mut self, now: i64) -> Result<Vec<DispatchedRun>, String> {
        let mut state = self.load_state()?;
        let mut dispatched = Vec::new();

        for mut job in self.store.list_all()? {
            if job.state != JobState::Enabled {
                continue;
            }
            let Some(next_run_at) = job.next_run_at else {
                continue;
            };
            if next_run_at > now {
                continue;
            }

            if next_run_at < now && matches!(job.misfire_policy, MisfirePolicy::SkipMissed) {
                job.next_run_at = compute_recovery_next_run_at(&job.schedule, now)?;
                if job.next_run_at.is_none() {
                    job.state = JobState::Disabled;
                }
                job.updated_at = now;
                job.last_result = Some("skipped_missed".to_string());
                self.store.persist_job(&job)?;
                self.persist_run_record(
                    &job,
                    &format!("skip-{}", now_nanos().abs()),
                    now,
                    RunStatus::SkippedMissed,
                    now,
                )?;
                self.append_scheduler_event("scheduler.misfire.skip_missed", &job, None, now);
                continue;
            }

            if !job.allow_overlap
                && state
                    .active_executions
                    .iter()
                    .any(|active| active.job_id == job.job_id)
            {
                continue;
            }

            let execution_id = format!("exec-{}-{}", sanitize_id(&job.job_id), next_run_at);
            if state
                .recent_execution_ids
                .iter()
                .any(|existing| existing == &execution_id)
                || self.execution_exists(&job.job_id, &execution_id)?
            {
                self.persist_run_record(
                    &job,
                    &execution_id,
                    now,
                    RunStatus::SkippedDuplicate,
                    now,
                )?;
                continue;
            }

            let envelope = ScheduledTriggerEnvelope {
                job_id: job.job_id.clone(),
                execution_id: execution_id.clone(),
                triggered_at: now,
                orchestrator_id: job.orchestrator_id.clone(),
                target_action: job.target_action.clone(),
                target_ref: job.target_ref.clone(),
            };
            self.enqueue_trigger(&envelope)?;
            self.append_scheduler_event(
                "scheduler.trigger.dispatched",
                &job,
                Some(&execution_id),
                now,
            );

            job.last_run_at = Some(now);
            job.last_result = Some("dispatched".to_string());
            if next_run_at < now && matches!(job.misfire_policy, MisfirePolicy::FireOnceOnRecovery)
            {
                self.append_scheduler_event("scheduler.misfire.fire_once", &job, None, now);
                job.next_run_at = compute_recovery_next_run_at(&job.schedule, now)?;
            } else {
                job.next_run_at = compute_next_run_at(&job.schedule, now, Some(next_run_at))?;
            }
            if job.next_run_at.is_none() {
                job.state = JobState::Disabled;
                job.last_result = Some("completed".to_string());
            }
            job.updated_at = now;
            self.store.persist_job(&job)?;
            self.persist_run_record(&job, &execution_id, now, RunStatus::Dispatched, now)?;

            state.recent_execution_ids.push(execution_id.clone());
            if state.recent_execution_ids.len() > 2048 {
                let start = state.recent_execution_ids.len() - 2048;
                state.recent_execution_ids = state.recent_execution_ids[start..].to_vec();
            }
            state.active_executions.push(ActiveExecution {
                job_id: job.job_id.clone(),
                execution_id: execution_id.clone(),
                started_at: now,
            });

            dispatched.push(DispatchedRun {
                job_id: job.job_id.clone(),
                execution_id,
                triggered_at: now,
            });
        }

        state.last_tick_at = Some(now);
        self.save_state(&state)?;
        Ok(dispatched)
    }

    pub fn complete_execution(
        &mut self,
        job_id: &str,
        execution_id: &str,
        succeeded: bool,
        now: i64,
    ) -> Result<(), String> {
        let mut state = self.load_state()?;
        state
            .active_executions
            .retain(|active| !(active.job_id == job_id && active.execution_id == execution_id));
        self.save_state(&state)?;

        if let Ok(mut job) = self.store.load(job_id) {
            let status = if succeeded { "succeeded" } else { "failed" };
            job.last_result = Some(status.to_string());
            job.updated_at = now;
            self.store.persist_job(&job)?;
            self.persist_run_record(
                &job,
                execution_id,
                now,
                if succeeded {
                    RunStatus::Completed
                } else {
                    RunStatus::Failed
                },
                now,
            )?;
            self.append_scheduler_event(
                if succeeded {
                    "scheduler.trigger.completed"
                } else {
                    "scheduler.trigger.failed"
                },
                &job,
                Some(execution_id),
                now,
            );
        } else {
            let mut payload = Map::new();
            payload.insert(
                "event".to_string(),
                Value::String(
                    if succeeded {
                        "scheduler.trigger.completed"
                    } else {
                        "scheduler.trigger.failed"
                    }
                    .to_string(),
                ),
            );
            payload.insert("jobId".to_string(), Value::String(job_id.to_string()));
            payload.insert(
                "executionId".to_string(),
                Value::String(execution_id.to_string()),
            );
            payload.insert("timestamp".to_string(), Value::from(now));
            if let Ok(line) = serde_json::to_string(&Value::Object(payload)) {
                let _ = append_orchestrator_log_line(&self.runtime_root, &line);
            }
        }
        Ok(())
    }

    fn enqueue_trigger(&self, envelope: &ScheduledTriggerEnvelope) -> Result<(), String> {
        let queue_paths = QueuePaths::from_state_root(&self.runtime_root);
        fs::create_dir_all(&queue_paths.incoming)
            .map_err(|err| format!("failed to create {}: {err}", queue_paths.incoming.display()))?;

        let incoming = IncomingMessage {
            channel: "scheduler".to_string(),
            channel_profile_id: None,
            sender: format!("scheduler:{}", envelope.orchestrator_id),
            sender_id: envelope.job_id.clone(),
            message: serde_json::to_string(envelope)
                .map_err(|err| format!("failed to encode scheduler envelope: {err}"))?,
            timestamp: envelope.triggered_at,
            message_id: envelope.execution_id.clone(),
            conversation_id: Some(format!("scheduler:{}", envelope.job_id)),
            files: Vec::new(),
            workflow_run_id: None,
            workflow_step_id: None,
        };

        let path = queue_paths
            .incoming
            .join(format!("{}.json", envelope.execution_id));
        let body = serde_json::to_vec_pretty(&incoming)
            .map_err(|err| format!("failed to encode queue payload: {err}"))?;
        fs::write(&path, body).map_err(|err| format!("failed to write {}: {err}", path.display()))
    }

    fn persist_run_record(
        &self,
        job: &ScheduledJob,
        execution_id: &str,
        triggered_at: i64,
        status: RunStatus,
        now: i64,
    ) -> Result<(), String> {
        let dir = self.runtime_root.join("automation/runs").join(&job.job_id);
        fs::create_dir_all(&dir)
            .map_err(|err| format!("failed to create {}: {err}", dir.display()))?;
        let path = dir.join(format!(
            "{}-{}.json",
            triggered_at,
            sanitize_id(execution_id)
        ));
        let record = RunRecord {
            job_id: job.job_id.clone(),
            execution_id: execution_id.to_string(),
            triggered_at,
            status,
            created_at: now,
        };
        let body = serde_json::to_vec_pretty(&record)
            .map_err(|err| format!("failed to encode run history: {err}"))?;
        fs::write(&path, body).map_err(|err| format!("failed to write {}: {err}", path.display()))
    }

    fn execution_exists(&self, job_id: &str, execution_id: &str) -> Result<bool, String> {
        let dir = self.runtime_root.join("automation/runs").join(job_id);
        if !dir.exists() {
            return Ok(false);
        }
        for entry in
            fs::read_dir(&dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?
        {
            let path = entry.map_err(|err| err.to_string())?.path();
            if path.extension().and_then(|v| v.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read_to_string(&path)
                .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
            let record: RunRecord = serde_json::from_str(&raw)
                .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
            if record.execution_id == execution_id {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn scheduler_state_path(&self) -> PathBuf {
        self.runtime_root.join("automation/scheduler_state.json")
    }

    fn load_state(&self) -> Result<SchedulerState, String> {
        let path = self.scheduler_state_path();
        if !path.exists() {
            return Ok(SchedulerState::default());
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|err| format!("failed to parse {}: {err}", path.display()))
    }

    fn save_state(&self, state: &SchedulerState) -> Result<(), String> {
        let path = self.scheduler_state_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        let body = serde_json::to_vec_pretty(state)
            .map_err(|err| format!("failed to encode scheduler state: {err}"))?;
        fs::write(&path, body).map_err(|err| format!("failed to write {}: {err}", path.display()))
    }

    fn append_scheduler_event(
        &self,
        event: &str,
        job: &ScheduledJob,
        execution_id: Option<&str>,
        now: i64,
    ) {
        let mut payload = Map::new();
        payload.insert("event".to_string(), Value::String(event.to_string()));
        payload.insert("jobId".to_string(), Value::String(job.job_id.clone()));
        payload.insert(
            "orchestratorId".to_string(),
            Value::String(job.orchestrator_id.clone()),
        );
        payload.insert("timestamp".to_string(), Value::from(now));
        if let Some(execution_id) = execution_id {
            payload.insert(
                "executionId".to_string(),
                Value::String(execution_id.to_string()),
            );
        }
        if let Ok(line) = serde_json::to_string(&Value::Object(payload)) {
            let _ = append_orchestrator_log_line(&self.runtime_root, &line);
        }
    }
}

pub fn complete_scheduled_execution(
    runtime_root: impl AsRef<Path>,
    job_id: &str,
    execution_id: &str,
    succeeded: bool,
    now: i64,
) -> Result<(), String> {
    let mut worker = SchedulerWorker::new(runtime_root);
    worker.complete_execution(job_id, execution_id, succeeded, now)
}

pub fn parse_trigger_envelope(message: &str) -> Result<ScheduledTriggerEnvelope, String> {
    serde_json::from_str(message).map_err(|err| format!("invalid scheduler trigger payload: {err}"))
}

pub fn validate_iana_timezone(raw: &str) -> Result<(), String> {
    raw.parse::<Tz>()
        .map(|_| ())
        .map_err(|_| format!("invalid timezone `{raw}`; expected IANA timezone id"))
}

#[derive(Debug, Clone)]
struct CronField {
    any: bool,
    values: BTreeSet<u32>,
}

impl CronField {
    fn matches(&self, value: u32) -> bool {
        self.any || self.values.contains(&value)
    }
}

#[derive(Debug, Clone)]
pub struct CronExpression {
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
}

pub fn parse_cron_expression(raw: &str) -> Result<CronExpression, String> {
    let fields: Vec<&str> = raw.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(
            "cron expression must use 5 fields: minute hour day_of_month month day_of_week"
                .to_string(),
        );
    }

    Ok(CronExpression {
        minute: parse_cron_field(fields[0], 0, 59, AliasKind::None)?,
        hour: parse_cron_field(fields[1], 0, 23, AliasKind::None)?,
        day_of_month: parse_cron_field(fields[2], 1, 31, AliasKind::None)?,
        month: parse_cron_field(fields[3], 1, 12, AliasKind::Month)?,
        day_of_week: parse_cron_field(fields[4], 0, 7, AliasKind::Weekday)?,
    })
}

fn cron_matches(expr: &CronExpression, unix_ts: i64, timezone: &Tz) -> bool {
    let Some(utc_dt) = Utc.timestamp_opt(unix_ts, 0).single() else {
        return false;
    };
    let local = utc_dt.with_timezone(timezone);

    if !expr.minute.matches(local.minute())
        || !expr.hour.matches(local.hour())
        || !expr.month.matches(local.month())
    {
        return false;
    }

    let day_of_month_match = expr.day_of_month.matches(local.day());
    let day_of_week = local.weekday().num_days_from_sunday();
    let day_of_week_match = expr.day_of_week.matches(day_of_week);

    if expr.day_of_month.any || expr.day_of_week.any {
        day_of_month_match && day_of_week_match
    } else {
        day_of_month_match || day_of_week_match
    }
}

pub fn compute_next_run_at(
    schedule: &ScheduleConfig,
    now: i64,
    last_run_at: Option<i64>,
) -> Result<Option<i64>, String> {
    match schedule {
        ScheduleConfig::Once { run_at } => {
            if last_run_at.is_some() {
                Ok(None)
            } else {
                Ok(Some(*run_at))
            }
        }
        ScheduleConfig::Interval {
            every_seconds,
            anchor_at,
        } => {
            if *every_seconds == 0 {
                return Err("interval.every_seconds must be >= 1".to_string());
            }
            let base = if let Some(last) = last_run_at {
                last.saturating_add(*every_seconds as i64)
            } else {
                anchor_at
                    .unwrap_or(now)
                    .saturating_add(*every_seconds as i64)
            };
            Ok(Some(base))
        }
        ScheduleConfig::Cron {
            expression,
            timezone,
        } => {
            let tz = timezone
                .parse::<Tz>()
                .map_err(|_| format!("invalid timezone `{timezone}`; expected IANA timezone id"))?;
            let cron = parse_cron_expression(expression)?;
            let mut candidate = ((last_run_at.unwrap_or(now) / 60) + 1) * 60;
            for _ in 0..MAX_CRON_SEARCH_MINUTES {
                if cron_matches(&cron, candidate, &tz) {
                    return Ok(Some(candidate));
                }
                candidate = candidate.saturating_add(60);
            }
            Err(format!(
                "unable to compute next run for cron expression `{expression}` in timezone `{timezone}`"
            ))
        }
    }
}

fn compute_recovery_next_run_at(
    schedule: &ScheduleConfig,
    now: i64,
) -> Result<Option<i64>, String> {
    match schedule {
        ScheduleConfig::Once { .. } => Ok(None),
        ScheduleConfig::Interval { .. } | ScheduleConfig::Cron { .. } => {
            compute_next_run_at(schedule, now, Some(now))
        }
    }
}

fn validate_schedule(schedule: &ScheduleConfig) -> Result<(), String> {
    match schedule {
        ScheduleConfig::Once { .. } => Ok(()),
        ScheduleConfig::Interval { every_seconds, .. } => {
            if *every_seconds == 0 {
                return Err("interval.every_seconds must be >= 1".to_string());
            }
            if *every_seconds > 31_536_000 {
                return Err("interval.every_seconds must be <= 31536000".to_string());
            }
            Ok(())
        }
        ScheduleConfig::Cron {
            expression,
            timezone,
        } => {
            parse_cron_expression(expression)?;
            validate_iana_timezone(timezone)
        }
    }
}

fn validate_target_action(action: &TargetAction) -> Result<(), String> {
    match action {
        TargetAction::WorkflowStart { workflow_id, .. } => {
            if workflow_id.trim().is_empty() {
                return Err(
                    "target_action.workflow_start.workflow_id must be non-empty".to_string()
                );
            }
            Ok(())
        }
        TargetAction::CommandInvoke { function_id, .. } => {
            if function_id.trim().is_empty() {
                return Err(
                    "target_action.command_invoke.function_id must be non-empty".to_string()
                );
            }
            Ok(())
        }
    }
}

fn valid_transition(from: JobState, to: JobState) -> bool {
    match from {
        JobState::Enabled => matches!(
            to,
            JobState::Paused | JobState::Disabled | JobState::Deleted
        ),
        JobState::Paused => matches!(
            to,
            JobState::Enabled | JobState::Disabled | JobState::Deleted
        ),
        JobState::Disabled => matches!(to, JobState::Enabled | JobState::Deleted),
        JobState::Deleted => false,
    }
}

fn state_name(state: JobState) -> &'static str {
    match state {
        JobState::Enabled => "enabled",
        JobState::Paused => "paused",
        JobState::Disabled => "disabled",
        JobState::Deleted => "deleted",
    }
}

fn sanitize_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
}

fn now_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AliasKind {
    None,
    Month,
    Weekday,
}

fn parse_cron_field(
    raw: &str,
    min: u32,
    max: u32,
    aliases: AliasKind,
) -> Result<CronField, String> {
    if raw == "*" {
        return Ok(CronField {
            any: true,
            values: BTreeSet::new(),
        });
    }

    let mut values = BTreeSet::new();
    for segment in raw.split(',') {
        parse_cron_segment(segment, min, max, aliases, &mut values)?;
    }
    if values.is_empty() {
        return Err(format!("invalid cron field `{raw}`"));
    }
    Ok(CronField { any: false, values })
}

fn parse_cron_segment(
    raw: &str,
    min: u32,
    max: u32,
    aliases: AliasKind,
    values: &mut BTreeSet<u32>,
) -> Result<(), String> {
    let (range_raw, step) = match raw.split_once('/') {
        Some((range, step_raw)) => {
            let step = step_raw
                .parse::<u32>()
                .map_err(|_| format!("invalid cron step `{step_raw}`"))?;
            if step == 0 {
                return Err("cron step must be >= 1".to_string());
            }
            (range, step)
        }
        None => (raw, 1),
    };

    let (start, end) = if range_raw == "*" {
        (min, max)
    } else if let Some((start_raw, end_raw)) = range_raw.split_once('-') {
        (
            parse_cron_atom(start_raw, min, max, aliases)?,
            parse_cron_atom(end_raw, min, max, aliases)?,
        )
    } else {
        let value = parse_cron_atom(range_raw, min, max, aliases)?;
        (value, value)
    };

    if start > end {
        return Err(format!("invalid cron range `{raw}`"));
    }

    let mut value = start;
    while value <= end {
        let normalized = if aliases == AliasKind::Weekday && value == 7 {
            0
        } else {
            value
        };
        values.insert(normalized);
        match value.checked_add(step) {
            Some(next) => value = next,
            None => break,
        }
    }
    Ok(())
}

fn parse_cron_atom(raw: &str, min: u32, max: u32, aliases: AliasKind) -> Result<u32, String> {
    let lower = raw.to_ascii_lowercase();
    let value = match aliases {
        AliasKind::None => lower
            .parse::<u32>()
            .map_err(|_| format!("invalid cron value `{raw}`"))?,
        AliasKind::Month => match lower.as_str() {
            "jan" => 1,
            "feb" => 2,
            "mar" => 3,
            "apr" => 4,
            "may" => 5,
            "jun" => 6,
            "jul" => 7,
            "aug" => 8,
            "sep" => 9,
            "oct" => 10,
            "nov" => 11,
            "dec" => 12,
            _ => lower
                .parse::<u32>()
                .map_err(|_| format!("invalid cron value `{raw}`"))?,
        },
        AliasKind::Weekday => match lower.as_str() {
            "sun" => 0,
            "mon" => 1,
            "tue" => 2,
            "wed" => 3,
            "thu" => 4,
            "fri" => 5,
            "sat" => 6,
            _ => lower
                .parse::<u32>()
                .map_err(|_| format!("invalid cron value `{raw}`"))?,
        },
    };

    if value < min || value > max {
        return Err(format!(
            "cron value `{raw}` is out of bounds ({min}..={max})"
        ));
    }
    Ok(value)
}
