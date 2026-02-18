use crate::app::command_support::{load_settings, now_secs};
use crate::app::schedule_parsing::{
    normalize_patch_slack_target_ref, normalize_slack_target_ref_value, parse_job_patch,
    parse_schedule_config, parse_schedule_create_tail_args, parse_target_action_config,
};
use crate::orchestration::scheduler::{JobPatch, JobStore, NewJob};
use crate::orchestration::slack_target::validate_profile_mapping;
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
                return Err("usage: schedule create <orchestrator_id> <schedule_type> <schedule_json> <target_action_json> [target_ref_json] [misfire_policy] [allow_overlap] [--target-ref <json>] [--misfire-policy <value>] [--allow-overlap <true|false>]".to_string());
            }
            let settings = load_settings()?;
            let orchestrator_id = args[1].clone();
            let schedule_type = args[2].to_ascii_lowercase();
            let schedule_obj = parse_json_object(&args[3], "schedule_json")?;
            let target_obj = parse_json_object(&args[4], "target_action_json")?;

            let schedule = parse_schedule_config(&schedule_type, &schedule_obj)?;
            let target_action = parse_target_action_config(&target_obj)?;
            let tail = parse_schedule_create_tail_args(&args[5..])?;
            let (target_ref, slack_target_ref) =
                normalize_slack_target_ref_value(tail.target_ref, "target_ref_json")?;
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
                    misfire_policy: tail.misfire_policy,
                    allow_overlap: tail.allow_overlap,
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

fn parse_patch(raw: &str) -> Result<JobPatch, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|err| format!("patch must be JSON: {err}"))?;
    let obj = value
        .as_object()
        .cloned()
        .ok_or_else(|| "patch must be JSON object".to_string())?;
    parse_job_patch(&obj)
}
