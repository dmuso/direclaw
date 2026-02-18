use direclaw::orchestration::scheduler::{
    compute_next_run_at, parse_cron_expression, validate_iana_timezone, JobPatch, JobState,
    JobStore, MisfirePolicy, NewJob, ScheduleConfig, TargetAction,
};
use serde_json::{Map, Value};
use tempfile::tempdir;

fn sample_create(now: i64) -> NewJob {
    NewJob {
        orchestrator_id: "eng".to_string(),
        created_by: Map::from_iter([
            (
                "channelProfileId".to_string(),
                Value::String("engineering".to_string()),
            ),
            ("senderId".to_string(), Value::String("U42".to_string())),
            ("sender".to_string(), Value::String("Dana".to_string())),
        ]),
        schedule: ScheduleConfig::Interval {
            every_seconds: 300,
            anchor_at: Some(now),
        },
        target_action: TargetAction::CommandInvoke {
            function_id: "workflow.status".to_string(),
            function_args: Map::from_iter([(
                "runId".to_string(),
                Value::String("run-1".to_string()),
            )]),
        },
        target_ref: None,
        misfire_policy: MisfirePolicy::FireOnceOnRecovery,
        allow_overlap: false,
    }
}

#[test]
fn scheduler_domain_validates_cron_and_timezones() {
    parse_cron_expression("*/15 9-17 * * mon-fri").expect("valid cron");
    parse_cron_expression("* * *").expect_err("invalid field count");

    validate_iana_timezone("America/Los_Angeles").expect("valid timezone");
    validate_iana_timezone("Mars/Olympus_Mons").expect_err("invalid timezone");
}

#[test]
fn scheduler_domain_persists_lifecycle_and_state_transitions() {
    let temp = tempdir().expect("tempdir");
    let now = 1_700_000_000_i64;
    let store = JobStore::new(temp.path());

    let created = store.create(sample_create(now), now).expect("create job");
    assert_eq!(created.state, JobState::Enabled);
    assert_eq!(created.next_run_at, Some(now + 300));

    let paused = store.pause(&created.job_id, now + 1).expect("pause");
    assert_eq!(paused.state, JobState::Paused);

    let resumed = store.resume(&created.job_id, now + 2).expect("resume");
    assert_eq!(resumed.state, JobState::Enabled);

    let updated = store
        .update(
            &created.job_id,
            JobPatch {
                schedule: Some(ScheduleConfig::Once { run_at: now + 30 }),
                target_action: None,
                target_ref: Some(Some(Value::Object(Map::from_iter([(
                    "channel".to_string(),
                    Value::String("C123".to_string()),
                )])))),
                misfire_policy: Some(MisfirePolicy::SkipMissed),
                allow_overlap: Some(true),
            },
            now + 3,
        )
        .expect("update");
    assert_eq!(updated.next_run_at, Some(now + 30));
    assert!(updated.allow_overlap);

    let deleted = store.delete(&created.job_id, now + 4).expect("delete");
    assert_eq!(deleted.state, JobState::Deleted);

    store
        .resume(&created.job_id, now + 5)
        .expect_err("cannot resume deleted");
}

#[test]
fn scheduler_domain_computes_next_run_for_once_interval_and_cron() {
    let now = 1_700_000_000_i64;

    let once = compute_next_run_at(&ScheduleConfig::Once { run_at: now + 60 }, now, None)
        .expect("once next run");
    assert_eq!(once, Some(now + 60));

    let interval = compute_next_run_at(
        &ScheduleConfig::Interval {
            every_seconds: 60,
            anchor_at: Some(now),
        },
        now + 181,
        Some(now + 120),
    )
    .expect("interval next run");
    assert_eq!(interval, Some(now + 180));

    let cron = compute_next_run_at(
        &ScheduleConfig::Cron {
            expression: "*/10 * * * *".to_string(),
            timezone: "UTC".to_string(),
        },
        1_700_000_000,
        None,
    )
    .expect("cron next run");
    assert!(cron.is_some());
}

#[test]
fn scheduler_domain_rejects_invalid_slack_target_ref_contract() {
    let temp = tempdir().expect("tempdir");
    let now = 1_700_000_000_i64;
    let store = JobStore::new(temp.path());

    let err = store
        .create(
            NewJob {
                target_ref: Some(Value::Object(Map::from_iter([
                    ("channel".to_string(), Value::String("slack".to_string())),
                    ("channelId".to_string(), Value::String("C123".to_string())),
                ]))),
                ..sample_create(now)
            },
            now,
        )
        .expect_err("invalid slack targetRef should fail");
    assert!(
        err.contains("target_ref.channelProfileId"),
        "unexpected error: {err}"
    );
}
