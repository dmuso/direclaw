use crate::app::command_support::{load_settings, now_secs};
use crate::orchestration::scheduler::{
    JobPatch, JobStore, MisfirePolicy, NewJob, ScheduleConfig, TargetAction,
};
use crate::orchestration::slack_target::{
    parse_slack_target_ref, slack_target_ref_to_value, validate_profile_mapping, SlackTargetRef,
};
use serde_json::{Map, Value};

pub fn cmd_schedule(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err(
            "usage: schedule <create|list|show|update|pause|resume|delete|run-now> ...".to_string(),
        );
    }

    match args[0].as_str() {
        "create" => {
            if args.len() < 5 {
                return Err("usage: schedule create <orchestrator_id> <schedule_type> <schedule_json> <target_action_json> [target_ref_json] [misfire_policy] [allow_overlap]".to_string());
            }
            let settings = load_settings()?;
            let orchestrator_id = args[1].clone();
            let schedule_type = args[2].to_ascii_lowercase();
            let schedule_obj = parse_json_object(&args[3], "schedule_json")?;
            let target_obj = parse_json_object(&args[4], "target_action_json")?;

            let mut schedule = parse_schedule_with_type(&schedule_type, &schedule_obj)?;
            let mut target_action = parse_target_action(&target_obj)?;

            let target_ref = if let Some(raw) = args.get(5) {
                Some(parse_json_object(raw, "target_ref_json")?.into())
            } else {
                None
            };
            let misfire_policy = if let Some(raw) = args.get(6) {
                parse_misfire_policy(raw)?
            } else {
                MisfirePolicy::FireOnceOnRecovery
            };
            let allow_overlap = if let Some(raw) = args.get(7) {
                parse_bool(raw, "allow_overlap")?
            } else {
                false
            };

            normalize_schedule_for_type(&mut schedule, &schedule_type)?;
            normalize_target_action(&mut target_action)?;
            let (target_ref, slack_target_ref) =
                normalize_slack_target_ref_value(target_ref, "target_ref_json")?;
            validate_profile_mapping(&settings, &orchestrator_id, slack_target_ref.as_ref())?;

            let runtime_root = settings
                .resolve_orchestrator_runtime_root(&orchestrator_id)
                .map_err(|err| err.to_string())?;
            let store = JobStore::new(runtime_root);
            let created = store.create(
                NewJob {
                    orchestrator_id,
                    created_by: Map::new(),
                    schedule,
                    target_action,
                    target_ref,
                    misfire_policy,
                    allow_overlap,
                },
                now_secs(),
            )?;

            Ok(format!(
                "schedule created\njob_id={}\nstate={:?}\nnext_run_at={}",
                created.job_id,
                created.state,
                created
                    .next_run_at
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ))
        }
        "list" => {
            if args.len() != 2 {
                return Err("usage: schedule list <orchestrator_id>".to_string());
            }
            let settings = load_settings()?;
            let orchestrator_id = &args[1];
            let runtime_root = settings
                .resolve_orchestrator_runtime_root(orchestrator_id)
                .map_err(|err| err.to_string())?;
            let store = JobStore::new(runtime_root);
            let jobs = store.list_for_orchestrator(orchestrator_id)?;
            if jobs.is_empty() {
                return Ok(String::new());
            }
            Ok(jobs
                .into_iter()
                .map(|job| {
                    format!(
                        "{}\t{:?}\t{}",
                        job.job_id,
                        job.state,
                        job.next_run_at
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "none".to_string())
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "show" => {
            if args.len() != 3 {
                return Err("usage: schedule show <orchestrator_id> <job_id>".to_string());
            }
            let settings = load_settings()?;
            let runtime_root = settings
                .resolve_orchestrator_runtime_root(&args[1])
                .map_err(|err| err.to_string())?;
            let store = JobStore::new(runtime_root);
            let job = store.load(&args[2])?;
            serde_json::to_string_pretty(&job)
                .map_err(|err| format!("failed to encode scheduler job: {err}"))
        }
        "update" => {
            if args.len() != 4 {
                return Err(
                    "usage: schedule update <orchestrator_id> <job_id> <patch_json>".to_string(),
                );
            }
            let settings = load_settings()?;
            let runtime_root = settings
                .resolve_orchestrator_runtime_root(&args[1])
                .map_err(|err| err.to_string())?;
            let store = JobStore::new(runtime_root);
            let mut patch = parse_patch(&args[3])?;
            let slack_target_ref = normalize_patch_slack_target_ref(&mut patch, "patch.targetRef")?;
            validate_profile_mapping(&settings, &args[1], slack_target_ref.as_ref())?;
            let job = store.update(&args[2], patch, now_secs())?;
            Ok(format!(
                "schedule updated\njob_id={}\nstate={:?}",
                job.job_id, job.state
            ))
        }
        "pause" => {
            if args.len() != 3 {
                return Err("usage: schedule pause <orchestrator_id> <job_id>".to_string());
            }
            let settings = load_settings()?;
            let runtime_root = settings
                .resolve_orchestrator_runtime_root(&args[1])
                .map_err(|err| err.to_string())?;
            let store = JobStore::new(runtime_root);
            let job = store.pause(&args[2], now_secs())?;
            Ok(format!(
                "schedule paused\njob_id={}\nstate={:?}",
                job.job_id, job.state
            ))
        }
        "resume" => {
            if args.len() != 3 {
                return Err("usage: schedule resume <orchestrator_id> <job_id>".to_string());
            }
            let settings = load_settings()?;
            let runtime_root = settings
                .resolve_orchestrator_runtime_root(&args[1])
                .map_err(|err| err.to_string())?;
            let store = JobStore::new(runtime_root);
            let job = store.resume(&args[2], now_secs())?;
            Ok(format!(
                "schedule resumed\njob_id={}\nstate={:?}",
                job.job_id, job.state
            ))
        }
        "delete" => {
            if args.len() != 3 {
                return Err("usage: schedule delete <orchestrator_id> <job_id>".to_string());
            }
            let settings = load_settings()?;
            let runtime_root = settings
                .resolve_orchestrator_runtime_root(&args[1])
                .map_err(|err| err.to_string())?;
            let store = JobStore::new(runtime_root);
            let job = store.delete(&args[2], now_secs())?;
            Ok(format!(
                "schedule deleted\njob_id={}\nstate={:?}",
                job.job_id, job.state
            ))
        }
        "run-now" => {
            if args.len() != 3 {
                return Err("usage: schedule run-now <orchestrator_id> <job_id>".to_string());
            }
            let settings = load_settings()?;
            let runtime_root = settings
                .resolve_orchestrator_runtime_root(&args[1])
                .map_err(|err| err.to_string())?;
            let store = JobStore::new(runtime_root);
            let job = store.run_now(&args[2], now_secs())?;
            Ok(format!(
                "schedule run_now set\njob_id={}\nnext_run_at={}",
                job.job_id,
                job.next_run_at
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ))
        }
        other => Err(format!("unknown schedule subcommand `{other}`")),
    }
}

fn parse_json_object(raw: &str, field: &str) -> Result<Map<String, Value>, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|err| format!("{field} must be valid JSON object: {err}"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{field} must be a JSON object"))
}

fn parse_schedule_with_type(
    schedule_type: &str,
    schedule: &Map<String, Value>,
) -> Result<ScheduleConfig, String> {
    match schedule_type {
        "once" => {
            let run_at = schedule
                .get("runAt")
                .and_then(Value::as_i64)
                .ok_or_else(|| "once schedule requires integer runAt".to_string())?;
            Ok(ScheduleConfig::Once { run_at })
        }
        "interval" => {
            let every_seconds = schedule
                .get("everySeconds")
                .and_then(Value::as_u64)
                .ok_or_else(|| "interval schedule requires integer everySeconds".to_string())?;
            let anchor_at = schedule.get("anchorAt").and_then(Value::as_i64);
            Ok(ScheduleConfig::Interval {
                every_seconds,
                anchor_at,
            })
        }
        "cron" => {
            let expression = schedule
                .get("expression")
                .and_then(Value::as_str)
                .ok_or_else(|| "cron schedule requires string expression".to_string())?
                .to_string();
            let timezone = schedule
                .get("timezone")
                .and_then(Value::as_str)
                .ok_or_else(|| "cron schedule requires string timezone".to_string())?
                .to_string();
            Ok(ScheduleConfig::Cron {
                expression,
                timezone,
            })
        }
        other => Err(format!(
            "schedule_type must be once|interval|cron, got `{other}`"
        )),
    }
}

fn parse_target_action(target: &Map<String, Value>) -> Result<TargetAction, String> {
    let action_type = target
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "target_action.type is required".to_string())?;

    match action_type {
        "workflow_start" => {
            let workflow_id = target
                .get("workflowId")
                .and_then(Value::as_str)
                .ok_or_else(|| "workflow_start requires workflowId".to_string())?
                .to_string();
            let inputs = target
                .get("inputs")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            Ok(TargetAction::WorkflowStart {
                workflow_id,
                inputs,
            })
        }
        "command_invoke" => {
            let function_id = target
                .get("functionId")
                .and_then(Value::as_str)
                .ok_or_else(|| "command_invoke requires functionId".to_string())?
                .to_string();
            let function_args = target
                .get("functionArgs")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            Ok(TargetAction::CommandInvoke {
                function_id,
                function_args,
            })
        }
        other => Err(format!(
            "target_action.type must be workflow_start|command_invoke, got `{other}`"
        )),
    }
}

fn parse_misfire_policy(raw: &str) -> Result<MisfirePolicy, String> {
    match raw {
        "fire_once_on_recovery" => Ok(MisfirePolicy::FireOnceOnRecovery),
        "skip_missed" => Ok(MisfirePolicy::SkipMissed),
        _ => Err("misfire_policy must be fire_once_on_recovery|skip_missed".to_string()),
    }
}

fn parse_bool(raw: &str, field: &str) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{field} must be true|false")),
    }
}

fn parse_patch(raw: &str) -> Result<JobPatch, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|err| format!("patch must be JSON: {err}"))?;
    let obj = value
        .as_object()
        .cloned()
        .ok_or_else(|| "patch must be JSON object".to_string())?;

    let schedule = obj
        .get("schedule")
        .and_then(Value::as_object)
        .map(|schedule_obj| {
            let schedule_type = schedule_obj
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| "patch.schedule.type is required".to_string())?;
            parse_schedule_with_type(schedule_type, schedule_obj)
        })
        .transpose()?;

    let target_action = obj
        .get("targetAction")
        .and_then(Value::as_object)
        .map(parse_target_action)
        .transpose()?;

    let misfire_policy = obj
        .get("misfirePolicy")
        .and_then(Value::as_str)
        .map(parse_misfire_policy)
        .transpose()?;

    let allow_overlap = obj.get("allowOverlap").and_then(Value::as_bool);
    let target_ref = if obj.contains_key("targetRef") {
        Some(obj.get("targetRef").cloned())
    } else {
        None
    };
    if let Some(Some(target_ref)) = target_ref.as_ref() {
        let _ = parse_slack_target_ref(target_ref, "patch.targetRef")?;
    }

    Ok(JobPatch {
        schedule,
        target_action,
        target_ref,
        misfire_policy,
        allow_overlap,
    })
}

fn normalize_schedule_for_type(
    schedule: &mut ScheduleConfig,
    schedule_type: &str,
) -> Result<(), String> {
    match (schedule_type, schedule) {
        ("once", ScheduleConfig::Once { .. }) => Ok(()),
        ("interval", ScheduleConfig::Interval { .. }) => Ok(()),
        ("cron", ScheduleConfig::Cron { .. }) => Ok(()),
        _ => Err("schedule_type and schedule payload are inconsistent".to_string()),
    }
}

fn normalize_target_action(_action: &mut TargetAction) -> Result<(), String> {
    Ok(())
}

fn normalize_slack_target_ref_value(
    target_ref: Option<Value>,
    field_path: &str,
) -> Result<(Option<Value>, Option<SlackTargetRef>), String> {
    let Some(target_ref) = target_ref else {
        return Ok((None, None));
    };
    let parsed = parse_slack_target_ref(&target_ref, field_path)?;
    if let Some(slack_target) = parsed {
        return Ok((
            Some(slack_target_ref_to_value(&slack_target)),
            Some(slack_target),
        ));
    }
    Ok((Some(target_ref), None))
}

fn normalize_patch_slack_target_ref(
    patch: &mut JobPatch,
    field_path: &str,
) -> Result<Option<SlackTargetRef>, String> {
    let Some(target_ref) = patch.target_ref.as_ref() else {
        return Ok(None);
    };
    let Some(target_ref) = target_ref.as_ref() else {
        return Ok(None);
    };
    let parsed = parse_slack_target_ref(target_ref, field_path)?;
    if let Some(slack_target) = parsed {
        patch.target_ref = Some(Some(slack_target_ref_to_value(&slack_target)));
        return Ok(Some(slack_target));
    }
    Ok(None)
}
