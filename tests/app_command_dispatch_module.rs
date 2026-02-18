use direclaw::app::command_catalog::function_ids;
use direclaw::app::command_dispatch::{
    execute_function_invocation_with_executor, execute_internal_function, FunctionExecutionContext,
    InternalFunction,
};
use direclaw::config::{AuthSyncConfig, Monitoring, Settings, SettingsOrchestrator};
use direclaw::memory::MemoryConfig;
use direclaw::orchestration::scheduler::{
    JobStore, MisfirePolicy, NewJob, ScheduleConfig, TargetAction,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use tempfile::tempdir;

#[test]
fn command_dispatch_module_executes_internal_orchestrator_list_function() {
    let temp = tempdir().expect("tempdir");
    let settings = Settings {
        workspaces_path: temp.path().to_path_buf(),
        shared_workspaces: BTreeMap::new(),
        orchestrators: BTreeMap::from_iter([(
            "main".to_string(),
            SettingsOrchestrator {
                private_workspace: None,
                shared_access: Vec::new(),
            },
        )]),
        channel_profiles: BTreeMap::new(),
        monitoring: Monitoring::default(),
        channels: BTreeMap::new(),
        auth_sync: AuthSyncConfig::default(),
        memory: MemoryConfig::default(),
    };

    let value = execute_function_invocation_with_executor(
        function_ids::ORCHESTRATOR_LIST,
        &Map::new(),
        FunctionExecutionContext {
            run_store: None,
            settings: Some(&settings),
            orchestrator_id: None,
        },
        |_| panic!("internal function should not invoke CLI executor"),
    )
    .expect("internal function result");

    let orchestrators = value
        .get("orchestrators")
        .and_then(|v| v.as_array())
        .expect("orchestrators array");
    assert_eq!(orchestrators, &vec![Value::String("main".to_string())]);
}

#[test]
fn schedule_show_rejects_cross_orchestrator_job_access() {
    let temp = tempdir().expect("tempdir");
    let alpha_ws = temp.path().join("alpha");
    let beta_ws = temp.path().join("beta");
    let settings = Settings {
        workspaces_path: temp.path().to_path_buf(),
        shared_workspaces: BTreeMap::new(),
        orchestrators: BTreeMap::from_iter([
            (
                "alpha".to_string(),
                SettingsOrchestrator {
                    private_workspace: Some(alpha_ws.clone()),
                    shared_access: Vec::new(),
                },
            ),
            (
                "beta".to_string(),
                SettingsOrchestrator {
                    private_workspace: Some(beta_ws),
                    shared_access: Vec::new(),
                },
            ),
        ]),
        channel_profiles: BTreeMap::new(),
        monitoring: Monitoring::default(),
        channels: BTreeMap::new(),
        auth_sync: AuthSyncConfig::default(),
        memory: MemoryConfig::default(),
    };

    let alpha_runtime = settings
        .resolve_orchestrator_runtime_root("alpha")
        .expect("alpha root");
    let alpha_store = JobStore::new(alpha_runtime);
    let created = alpha_store
        .create(
            NewJob {
                orchestrator_id: "alpha".to_string(),
                created_by: Map::new(),
                schedule: ScheduleConfig::Interval {
                    every_seconds: 60,
                    anchor_at: Some(1_700_000_000),
                },
                target_action: TargetAction::CommandInvoke {
                    function_id: function_ids::ORCHESTRATOR_LIST.to_string(),
                    function_args: Map::new(),
                },
                target_ref: None,
                misfire_policy: MisfirePolicy::FireOnceOnRecovery,
                allow_overlap: false,
            },
            1_700_000_000,
        )
        .expect("create");

    let err = execute_internal_function(
        InternalFunction::ScheduleShow {
            job_id: created.job_id,
        },
        FunctionExecutionContext {
            run_store: None,
            settings: Some(&settings),
            orchestrator_id: Some("beta"),
        },
    )
    .expect_err("cross-orchestrator access should be denied");

    assert!(
        err.to_string().contains("unknown scheduler job"),
        "unexpected error: {err}"
    );
}

#[test]
fn schedule_lifecycle_emits_scheduler_audit_events() {
    let temp = tempdir().expect("tempdir");
    let main_ws = temp.path().join("main");
    let settings = Settings {
        workspaces_path: temp.path().to_path_buf(),
        shared_workspaces: BTreeMap::new(),
        orchestrators: BTreeMap::from_iter([(
            "main".to_string(),
            SettingsOrchestrator {
                private_workspace: Some(main_ws.clone()),
                shared_access: Vec::new(),
            },
        )]),
        channel_profiles: BTreeMap::new(),
        monitoring: Monitoring::default(),
        channels: BTreeMap::new(),
        auth_sync: AuthSyncConfig::default(),
        memory: MemoryConfig::default(),
    };

    let created = execute_internal_function(
        InternalFunction::ScheduleCreate {
            orchestrator_id: "main".to_string(),
            schedule: ScheduleConfig::Interval {
                every_seconds: 60,
                anchor_at: Some(1_700_000_000),
            },
            target_action: TargetAction::CommandInvoke {
                function_id: function_ids::ORCHESTRATOR_LIST.to_string(),
                function_args: Map::new(),
            },
            target_ref: None,
            misfire_policy: MisfirePolicy::FireOnceOnRecovery,
            allow_overlap: false,
            created_by: Map::new(),
        },
        FunctionExecutionContext {
            run_store: None,
            settings: Some(&settings),
            orchestrator_id: Some("main"),
        },
    )
    .expect("create");
    let job_id = created
        .get("jobId")
        .and_then(Value::as_str)
        .expect("job id")
        .to_string();

    execute_internal_function(
        InternalFunction::SchedulePause {
            job_id: job_id.clone(),
        },
        FunctionExecutionContext {
            run_store: None,
            settings: Some(&settings),
            orchestrator_id: Some("main"),
        },
    )
    .expect("pause");
    execute_internal_function(
        InternalFunction::ScheduleResume { job_id },
        FunctionExecutionContext {
            run_store: None,
            settings: Some(&settings),
            orchestrator_id: Some("main"),
        },
    )
    .expect("resume");

    let log_path = main_ws.join("logs/orchestrator.log");
    let log = std::fs::read_to_string(&log_path).expect("read log");
    assert!(
        log.contains("\"event\":\"scheduler.job.created\""),
        "missing created event in log: {log}"
    );
    assert!(
        log.contains("\"event\":\"scheduler.job.paused\""),
        "missing paused event in log: {log}"
    );
    assert!(
        log.contains("\"event\":\"scheduler.job.resumed\""),
        "missing resumed event in log: {log}"
    );
}
