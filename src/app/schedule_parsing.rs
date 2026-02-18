use crate::orchestration::scheduler::{JobPatch, MisfirePolicy, ScheduleConfig, TargetAction};
use crate::orchestration::slack_target::{
    parse_slack_target_ref, slack_target_ref_to_value, SlackTargetRef,
};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCreateTailArgs {
    pub target_ref: Option<Value>,
    pub misfire_policy: MisfirePolicy,
    pub allow_overlap: bool,
}

pub fn parse_schedule_config(
    schedule_type: &str,
    schedule: &Map<String, Value>,
) -> Result<ScheduleConfig, String> {
    match schedule_type {
        "once" => {
            let run_at = schedule
                .get("runAt")
                .and_then(Value::as_i64)
                .ok_or_else(|| "schedule.once requires integer `runAt`".to_string())?;
            Ok(ScheduleConfig::Once { run_at })
        }
        "interval" => {
            let every_seconds = schedule
                .get("everySeconds")
                .and_then(Value::as_u64)
                .ok_or_else(|| "schedule.interval requires integer `everySeconds`".to_string())?;
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
                .ok_or_else(|| "schedule.cron requires string `expression`".to_string())?
                .to_string();
            let timezone = schedule
                .get("timezone")
                .and_then(Value::as_str)
                .ok_or_else(|| "schedule.cron requires string `timezone`".to_string())?
                .to_string();
            Ok(ScheduleConfig::Cron {
                expression,
                timezone,
            })
        }
        other => Err(format!(
            "scheduleType must be one of: once, interval, cron (got `{other}`)"
        )),
    }
}

pub fn parse_target_action_config(action: &Map<String, Value>) -> Result<TargetAction, String> {
    let action_type = action
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "targetAction.type is required".to_string())?;
    match action_type {
        "workflow_start" => {
            let workflow_id = action
                .get("workflowId")
                .and_then(Value::as_str)
                .ok_or_else(|| "targetAction.workflow_start requires `workflowId`".to_string())?
                .to_string();
            let inputs = action
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
            let function_id = action
                .get("functionId")
                .and_then(Value::as_str)
                .ok_or_else(|| "targetAction.command_invoke requires `functionId`".to_string())?
                .to_string();
            let function_args = action
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
            "targetAction.type must be workflow_start|command_invoke (got `{other}`)"
        )),
    }
}

pub fn parse_misfire_policy_arg(raw: Option<&str>) -> Result<MisfirePolicy, String> {
    match raw {
        None => Ok(MisfirePolicy::FireOnceOnRecovery),
        Some("fire_once_on_recovery") => Ok(MisfirePolicy::FireOnceOnRecovery),
        Some("skip_missed") => Ok(MisfirePolicy::SkipMissed),
        Some(other) => Err(format!(
            "misfirePolicy must be fire_once_on_recovery|skip_missed (got `{other}`)"
        )),
    }
}

pub fn parse_schedule_create_tail_args(args: &[String]) -> Result<ScheduleCreateTailArgs, String> {
    if args.is_empty() {
        return Ok(ScheduleCreateTailArgs {
            target_ref: None,
            misfire_policy: MisfirePolicy::FireOnceOnRecovery,
            allow_overlap: false,
        });
    }
    if args.iter().any(|arg| arg.starts_with("--")) {
        return parse_schedule_create_tail_flags(args);
    }
    parse_schedule_create_tail_positionals(args)
}

fn parse_schedule_create_tail_flags(args: &[String]) -> Result<ScheduleCreateTailArgs, String> {
    let mut target_ref = None;
    let mut misfire_policy = MisfirePolicy::FireOnceOnRecovery;
    let mut allow_overlap = false;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--target-ref" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--target-ref requires a JSON object argument".to_string())?;
                target_ref = Some(parse_json_object_value(raw, "target_ref_json")?);
            }
            "--misfire-policy" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--misfire-policy requires a value".to_string())?;
                misfire_policy = parse_misfire_policy_arg(Some(raw.as_str()))?;
            }
            "--allow-overlap" => {
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--allow-overlap requires true|false".to_string())?;
                allow_overlap = parse_bool(raw, "allow_overlap")?;
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown schedule create flag `{other}`"));
            }
            other => {
                return Err(format!(
                    "unexpected positional argument `{other}` when using named schedule flags"
                ));
            }
        }
        index += 1;
    }
    Ok(ScheduleCreateTailArgs {
        target_ref,
        misfire_policy,
        allow_overlap,
    })
}

fn parse_schedule_create_tail_positionals(
    args: &[String],
) -> Result<ScheduleCreateTailArgs, String> {
    let mut target_ref = None;
    let mut misfire_policy = MisfirePolicy::FireOnceOnRecovery;
    let mut allow_overlap = false;
    let mut misfire_set = false;
    let mut overlap_set = false;

    for raw in args {
        if target_ref.is_none() {
            if let Ok(value) = parse_json_object_value(raw, "target_ref_json") {
                target_ref = Some(value);
                continue;
            }
        }
        if !misfire_set {
            if let Ok(policy) = parse_misfire_policy_arg(Some(raw.as_str())) {
                misfire_policy = policy;
                misfire_set = true;
                continue;
            }
        }
        if !overlap_set {
            if let Ok(value) = parse_bool(raw, "allow_overlap") {
                allow_overlap = value;
                overlap_set = true;
                continue;
            }
        }
        return Err(format!(
            "invalid schedule create argument `{raw}`; expected target_ref_json, misfire_policy, or allow_overlap"
        ));
    }

    Ok(ScheduleCreateTailArgs {
        target_ref,
        misfire_policy,
        allow_overlap,
    })
}

pub fn parse_job_patch(patch: &Map<String, Value>) -> Result<JobPatch, String> {
    let schedule = patch
        .get("schedule")
        .and_then(Value::as_object)
        .map(|schedule| {
            let schedule_type = schedule
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| "patch.schedule.type is required".to_string())?;
            parse_schedule_config(schedule_type, schedule)
        })
        .transpose()?;
    let target_action = patch
        .get("targetAction")
        .and_then(Value::as_object)
        .map(parse_target_action_config)
        .transpose()?;
    let target_ref = if patch.contains_key("targetRef") {
        Some(patch.get("targetRef").cloned())
    } else {
        None
    };
    if let Some(Some(target_ref)) = target_ref.as_ref() {
        let _ = parse_slack_target_ref(target_ref, "patch.targetRef")?;
    }
    let misfire_policy = match patch.get("misfirePolicy").and_then(Value::as_str) {
        Some(raw) => Some(parse_misfire_policy_arg(Some(raw))?),
        None => None,
    };
    let allow_overlap = patch.get("allowOverlap").and_then(Value::as_bool);

    Ok(JobPatch {
        schedule,
        target_action,
        target_ref,
        misfire_policy,
        allow_overlap,
    })
}

pub fn normalize_slack_target_ref_value(
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

pub fn normalize_patch_slack_target_ref(
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

fn parse_json_object_value(raw: &str, field: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|err| format!("{field} must be valid JSON object: {err}"))?;
    if !value.is_object() {
        return Err(format!("{field} must be a JSON object"));
    }
    Ok(value)
}

fn parse_bool(raw: &str, field: &str) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{field} must be true|false")),
    }
}
