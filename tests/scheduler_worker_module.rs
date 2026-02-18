use direclaw::orchestration::scheduler::{
    complete_scheduled_execution, JobStore, MisfirePolicy, NewJob, ScheduleConfig, SchedulerWorker,
    TargetAction,
};
use direclaw::queue::{claim_oldest, IncomingMessage, QueuePaths};
use serde_json::Map;
use tempfile::tempdir;

fn make_job(misfire_policy: MisfirePolicy) -> NewJob {
    NewJob {
        orchestrator_id: "eng".to_string(),
        created_by: Map::new(),
        schedule: ScheduleConfig::Once {
            run_at: 1_700_000_000,
        },
        target_action: TargetAction::WorkflowStart {
            workflow_id: "triage".to_string(),
            inputs: Map::new(),
        },
        target_ref: None,
        misfire_policy,
        allow_overlap: false,
    }
}

fn make_interval_job(misfire_policy: MisfirePolicy) -> NewJob {
    NewJob {
        orchestrator_id: "eng".to_string(),
        created_by: Map::new(),
        schedule: ScheduleConfig::Interval {
            every_seconds: 60,
            anchor_at: Some(1_700_000_000),
        },
        target_action: TargetAction::WorkflowStart {
            workflow_id: "triage".to_string(),
            inputs: Map::new(),
        },
        target_ref: None,
        misfire_policy,
        allow_overlap: false,
    }
}

#[test]
fn scheduler_worker_dispatches_due_job_through_queue_and_records_run_history() {
    let temp = tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime");
    let queue = QueuePaths::from_state_root(&runtime_root);
    std::fs::create_dir_all(&queue.incoming).expect("incoming dir");
    std::fs::create_dir_all(&queue.processing).expect("processing dir");

    let store = JobStore::new(&runtime_root);
    let created = store
        .create(make_job(MisfirePolicy::FireOnceOnRecovery), 1_700_000_000)
        .expect("create");

    let mut worker = SchedulerWorker::new(&runtime_root);
    let dispatched = worker.tick(1_700_000_001).expect("tick");
    assert_eq!(dispatched.len(), 1);
    assert_eq!(dispatched[0].job_id, created.job_id);

    let claimed = claim_oldest(&queue)
        .expect("claim")
        .expect("expected queued scheduled trigger");
    let inbound: IncomingMessage = claimed.payload;
    assert_eq!(inbound.channel, "scheduler");
    assert!(inbound.sender.starts_with("scheduler:"));
    assert!(inbound.message.contains("\"jobId\""));
    assert!(inbound.message.contains("\"executionId\""));
    assert!(inbound.message.contains("\"triggeredAt\""));

    let runs_dir = runtime_root.join(format!("automation/runs/{}/", created.job_id));
    assert!(runs_dir.exists(), "missing run history directory");
    let log = std::fs::read_to_string(runtime_root.join("logs/orchestrator.log")).expect("log");
    assert!(
        log.contains("\"event\":\"scheduler.trigger.dispatched\""),
        "missing trigger dispatch event in log: {log}"
    );
}

#[test]
fn scheduler_worker_applies_skip_missed_without_dispatching() {
    let temp = tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime");
    let queue = QueuePaths::from_state_root(&runtime_root);
    std::fs::create_dir_all(&queue.incoming).expect("incoming dir");
    std::fs::create_dir_all(&queue.processing).expect("processing dir");

    let store = JobStore::new(&runtime_root);
    let created = store
        .create(make_job(MisfirePolicy::SkipMissed), 1_700_000_000)
        .expect("create");

    let mut worker = SchedulerWorker::new(&runtime_root);
    let dispatched = worker.tick(1_700_010_000).expect("tick");
    assert!(dispatched.is_empty(), "skip_missed should not dispatch");

    let job = store.load(&created.job_id).expect("load");
    assert!(job.last_run_at.is_none());

    let claim = claim_oldest(&queue).expect("claim");
    assert!(claim.is_none(), "queue should stay empty");
    let log = std::fs::read_to_string(runtime_root.join("logs/orchestrator.log")).expect("log");
    assert!(
        log.contains("\"event\":\"scheduler.misfire.skip_missed\""),
        "missing misfire event in log: {log}"
    );
}

#[test]
fn scheduler_worker_suppresses_duplicate_execution_id() {
    let temp = tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime");
    std::fs::create_dir_all(runtime_root.join("queue/incoming")).expect("incoming dir");

    let store = JobStore::new(&runtime_root);
    let created = store
        .create(make_job(MisfirePolicy::FireOnceOnRecovery), 1_700_000_000)
        .expect("create");

    let mut worker = SchedulerWorker::new(&runtime_root);
    let first = worker.tick(1_700_000_001).expect("first tick");
    assert_eq!(first.len(), 1);

    let second = worker.tick(1_700_000_001).expect("second tick");
    assert!(second.is_empty());

    let runs_dir = runtime_root.join(format!("automation/runs/{}/", created.job_id));
    let entries = std::fs::read_dir(&runs_dir).expect("read runs").count();
    assert_eq!(entries, 1, "expected one run history entry");
}

#[test]
fn scheduler_worker_blocks_overlap_only_while_execution_is_active() {
    let temp = tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime");
    std::fs::create_dir_all(runtime_root.join("queue/incoming")).expect("incoming dir");

    let store = JobStore::new(&runtime_root);
    let created = store
        .create(
            make_interval_job(MisfirePolicy::FireOnceOnRecovery),
            1_700_000_000,
        )
        .expect("create");

    let mut worker = SchedulerWorker::new(&runtime_root);
    let first = worker.tick(1_700_000_060).expect("first tick");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].job_id, created.job_id);

    let blocked = worker.tick(1_700_000_120).expect("blocked tick");
    assert!(
        blocked.is_empty(),
        "overlap=false should block while prior execution is active"
    );

    complete_scheduled_execution(
        &runtime_root,
        &created.job_id,
        &first[0].execution_id,
        true,
        1_700_000_121,
    )
    .expect("complete execution");

    let second = worker.tick(1_700_000_120).expect("second tick");
    assert_eq!(second.len(), 1, "expected recurring run after completion");
}

#[test]
fn scheduler_worker_advances_interval_misfire_policies_to_future_slot() {
    let temp = tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime");
    std::fs::create_dir_all(runtime_root.join("queue/incoming")).expect("incoming dir");

    let store = JobStore::new(&runtime_root);
    let skip = store
        .create(make_interval_job(MisfirePolicy::SkipMissed), 1_700_000_000)
        .expect("create skip");
    let fire = store
        .create(
            make_interval_job(MisfirePolicy::FireOnceOnRecovery),
            1_700_000_000,
        )
        .expect("create fire");

    let mut worker = SchedulerWorker::new(&runtime_root);
    let dispatched = worker.tick(1_700_010_000).expect("tick");
    assert_eq!(
        dispatched.len(),
        1,
        "fire_once_on_recovery should dispatch exactly once"
    );

    let skip_job = store.load(&skip.job_id).expect("load skip");
    assert!(skip_job.next_run_at.expect("skip next") > 1_700_010_000);

    let fire_job = store.load(&fire.job_id).expect("load fire");
    assert!(fire_job.next_run_at.expect("fire next") > 1_700_010_000);
}

#[test]
fn scheduler_worker_advances_cron_skip_missed_to_future_slot() {
    let temp = tempdir().expect("tempdir");
    let runtime_root = temp.path().join("runtime");
    std::fs::create_dir_all(runtime_root.join("queue/incoming")).expect("incoming dir");

    let store = JobStore::new(&runtime_root);
    let created = store
        .create(
            NewJob {
                orchestrator_id: "eng".to_string(),
                created_by: Map::new(),
                schedule: ScheduleConfig::Cron {
                    expression: "*/5 * * * *".to_string(),
                    timezone: "UTC".to_string(),
                },
                target_action: TargetAction::WorkflowStart {
                    workflow_id: "triage".to_string(),
                    inputs: Map::new(),
                },
                target_ref: None,
                misfire_policy: MisfirePolicy::SkipMissed,
                allow_overlap: false,
            },
            1_700_000_000,
        )
        .expect("create");

    let mut worker = SchedulerWorker::new(&runtime_root);
    let dispatched = worker.tick(1_700_200_000).expect("tick");
    assert!(dispatched.is_empty(), "skip_missed should not dispatch");

    let job = store.load(&created.job_id).expect("load");
    assert!(job.next_run_at.expect("next run") > 1_700_200_000);
}
