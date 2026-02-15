use crate::config::{
    OutputContractKey, WorkflowConfig, WorkflowStepConfig, WorkflowStepPromptType, WorkflowStepType,
};
use crate::orchestration::workspace_access::normalize_absolute_path;
use crate::orchestrator::OrchestratorError;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

fn io_error(path: &Path, source: std::io::Error) -> OrchestratorError {
    OrchestratorError::Io {
        path: path.display().to_string(),
        source,
    }
}

pub fn parse_workflow_result_envelope(
    output: &str,
) -> Result<Map<String, Value>, OrchestratorError> {
    let open_tag = "[workflow_result]";
    let close_tag = "[/workflow_result]";
    let start = output.find(open_tag).ok_or_else(|| {
        OrchestratorError::WorkflowEnvelope("missing [workflow_result] tag".to_string())
    })?;
    let end = output.find(close_tag).ok_or_else(|| {
        OrchestratorError::WorkflowEnvelope("missing [/workflow_result] tag".to_string())
    })?;
    if output[start + open_tag.len()..].contains(open_tag) {
        return Err(OrchestratorError::WorkflowEnvelope(
            "multiple [workflow_result] tags are not allowed".to_string(),
        ));
    }
    if output[end + close_tag.len()..].contains(close_tag) {
        return Err(OrchestratorError::WorkflowEnvelope(
            "multiple [/workflow_result] tags are not allowed".to_string(),
        ));
    }
    if end <= start {
        return Err(OrchestratorError::WorkflowEnvelope(
            "invalid workflow_result tag ordering".to_string(),
        ));
    }
    let json_str = output[start + open_tag.len()..end].trim();
    let value: Value = serde_json::from_str(json_str)
        .map_err(|e| OrchestratorError::WorkflowEnvelope(format!("invalid json: {e}")))?;
    let obj = value.as_object().ok_or_else(|| {
        OrchestratorError::WorkflowEnvelope(
            "workflow_result payload must be a JSON object".to_string(),
        )
    })?;
    Ok(obj.clone())
}

pub fn parse_review_decision(outputs: &Map<String, Value>) -> Result<bool, OrchestratorError> {
    let decision = outputs
        .get("decision")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    match decision.to_ascii_lowercase().as_str() {
        "approve" => Ok(true),
        "reject" => Ok(false),
        other => Err(OrchestratorError::InvalidReviewDecision(other.to_string())),
    }
}

fn parse_output_contract(
    step: &WorkflowStepConfig,
) -> Result<Vec<OutputContractKey>, OrchestratorError> {
    Ok(step.outputs.clone())
}

fn validate_outputs_contract(
    step: &WorkflowStepConfig,
    outputs: &Map<String, Value>,
) -> Result<(), OrchestratorError> {
    let contract = parse_output_contract(step)?;
    if contract.is_empty() {
        return Ok(());
    }

    let mut missing_required = Vec::new();
    for key in contract.into_iter().filter(|key| key.required) {
        if !outputs.contains_key(&key.name) {
            missing_required.push(key.name);
        }
    }
    if missing_required.is_empty() {
        return Ok(());
    }

    missing_required.sort();
    let details = missing_required
        .iter()
        .map(|key| format!("{key}=missing"))
        .collect::<Vec<_>>()
        .join(", ");
    Err(OrchestratorError::OutputContractValidation {
        step_id: step.id.clone(),
        reason: format!("missing required output keys: {details}"),
    })
}

pub(crate) fn output_validation_errors_for(error: &OrchestratorError) -> BTreeMap<String, String> {
    match error {
        OrchestratorError::OutputContractValidation { reason, .. } => reason
            .trim()
            .strip_prefix("missing required output keys:")
            .unwrap_or(reason.as_str())
            .split(',')
            .filter_map(|entry| {
                let mut parts = entry.trim().splitn(2, '=');
                let key = parts.next()?.trim();
                let detail = parts.next()?.trim();
                if key.is_empty() || detail.is_empty() {
                    return None;
                }
                Some((key.to_string(), detail.to_string()))
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn validate_transition_target(
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
    target: Option<String>,
    reason: &str,
) -> Result<Option<String>, OrchestratorError> {
    let Some(target) = target else {
        return Ok(None);
    };
    if workflow
        .steps
        .iter()
        .any(|candidate| candidate.id == target)
    {
        return Ok(Some(target));
    }
    Err(OrchestratorError::TransitionValidation {
        step_id: step.id.clone(),
        reason: format!("{reason} targets unknown step `{target}`"),
    })
}

pub(crate) fn materialize_output_files(
    step: &WorkflowStepConfig,
    outputs: &Map<String, Value>,
    output_paths: &BTreeMap<String, PathBuf>,
) -> Result<BTreeMap<String, String>, OrchestratorError> {
    if output_paths.is_empty() {
        return Ok(BTreeMap::new());
    }

    let contract = parse_output_contract(step)?;
    let mut path_by_key = BTreeMap::new();
    for key in contract {
        let Some(value) = outputs.get(&key.name) else {
            continue;
        };
        let Some(path) = output_paths.get(&key.name) else {
            return Err(OrchestratorError::OutputContractValidation {
                step_id: step.id.clone(),
                reason: format!("missing output_files mapping for key `{}`", key.name),
            });
        };
        if step.prompt_type == WorkflowStepPromptType::WorkflowResultEnvelope {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
            }
            let content = if let Some(text) = value.as_str() {
                text.as_bytes().to_vec()
            } else {
                serde_json::to_vec_pretty(value).map_err(|e| OrchestratorError::StepExecution {
                    step_id: step.id.clone(),
                    reason: format!("failed to serialize output key `{}`: {e}", key.name),
                })?
            };
            fs::write(path, content).map_err(|e| io_error(path, e))?;
        }
        path_by_key.insert(key.name, path.display().to_string());
    }

    Ok(path_by_key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEvaluation {
    pub outputs: Map<String, Value>,
    pub output_files: BTreeMap<String, String>,
    pub next_step_id: Option<String>,
}

pub fn evaluate_step_result(
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
    raw_output: &str,
    output_paths: &BTreeMap<String, PathBuf>,
) -> Result<StepEvaluation, OrchestratorError> {
    let parsed = match step.prompt_type {
        WorkflowStepPromptType::WorkflowResultEnvelope => {
            parse_workflow_result_envelope(raw_output)?
        }
        WorkflowStepPromptType::FileOutput => load_outputs_from_files(step, output_paths)?,
    };
    validate_outputs_contract(step, &parsed)?;
    if step.step_type == WorkflowStepType::AgentReview {
        let approve = parse_review_decision(&parsed)?;
        let next = if approve {
            step.on_approve.clone()
        } else {
            step.on_reject.clone()
        };
        if next.is_none() {
            return Err(OrchestratorError::TransitionValidation {
                step_id: step.id.clone(),
                reason: if approve {
                    "decision `approve` requires `on_approve` transition target".to_string()
                } else {
                    "decision `reject` requires `on_reject` transition target".to_string()
                },
            });
        }
        let next = validate_transition_target(workflow, step, next, "review transition")?;
        return Ok(StepEvaluation {
            outputs: parsed,
            output_files: BTreeMap::new(),
            next_step_id: next,
        });
    }

    let next = step
        .next
        .clone()
        .or_else(|| next_step_in_workflow(workflow, &step.id));
    let next = validate_transition_target(workflow, step, next, "step transition")?;

    Ok(StepEvaluation {
        outputs: parsed,
        output_files: BTreeMap::new(),
        next_step_id: next,
    })
}

fn load_outputs_from_files(
    step: &WorkflowStepConfig,
    output_paths: &BTreeMap<String, PathBuf>,
) -> Result<Map<String, Value>, OrchestratorError> {
    let contract = parse_output_contract(step)?;
    if contract.is_empty() {
        return Ok(Map::new());
    }

    let mut outputs = Map::new();
    let mut missing_required = Vec::new();
    for key in contract {
        let Some(path) = output_paths.get(&key.name) else {
            return Err(OrchestratorError::OutputContractValidation {
                step_id: step.id.clone(),
                reason: format!("missing output_files mapping for key `{}`", key.name),
            });
        };
        if !path.is_file() {
            if key.required {
                missing_required.push(format!("{}=file_not_found", key.name));
            }
            continue;
        }
        let raw = fs::read_to_string(path).map_err(|err| io_error(path, err))?;
        let trimmed = raw.trim();
        let value = if trimmed.starts_with('{')
            || trimmed.starts_with('[')
            || trimmed.starts_with('"')
            || trimmed == "true"
            || trimmed == "false"
            || trimmed == "null"
            || trimmed.parse::<f64>().is_ok()
        {
            serde_json::from_str(trimmed).unwrap_or(Value::String(raw))
        } else {
            Value::String(raw)
        };
        outputs.insert(key.name, value);
    }

    if missing_required.is_empty() {
        return Ok(outputs);
    }
    missing_required.sort();
    Err(OrchestratorError::OutputContractValidation {
        step_id: step.id.clone(),
        reason: format!(
            "missing required output keys: {}",
            missing_required.join(", ")
        ),
    })
}

fn next_step_in_workflow(workflow: &WorkflowConfig, step_id: &str) -> Option<String> {
    workflow
        .steps
        .iter()
        .position(|s| s.id == step_id)
        .and_then(|idx| workflow.steps.get(idx + 1))
        .map(|s| s.id.clone())
}

pub fn interpolate_output_template(
    template: &str,
    run_id: &str,
    step_id: &str,
    attempt: u32,
) -> String {
    template
        .replace("{{workflow.run_id}}", run_id)
        .replace("{{workflow.step_id}}", step_id)
        .replace("{{workflow.attempt}}", &attempt.to_string())
}

pub fn resolve_step_output_paths(
    state_root: &Path,
    run_id: &str,
    step: &WorkflowStepConfig,
    attempt: u32,
) -> Result<BTreeMap<String, PathBuf>, OrchestratorError> {
    let output_root = normalize_absolute_path(
        &state_root
            .join("workflows/runs")
            .join(run_id)
            .join("steps")
            .join(&step.id)
            .join("attempts")
            .join(attempt.to_string())
            .join("outputs"),
    )?;

    let mut output_paths = BTreeMap::new();

    for (key, template) in &step.output_files {
        let interpolated =
            interpolate_output_template(template.as_str(), run_id, &step.id, attempt);
        let relative =
            validate_relative_output_template(&interpolated, &step.id, template.as_str())?;
        let resolved = normalize_absolute_path(&output_root.join(relative))?;
        if !resolved.starts_with(&output_root) {
            return Err(OrchestratorError::OutputPathValidation {
                step_id: step.id.clone(),
                template: template.to_string(),
                reason: format!(
                    "resolved path `{}` escapes output root `{}`",
                    resolved.display(),
                    output_root.display()
                ),
            });
        }
        output_paths.insert(key.as_str().to_string(), resolved);
    }
    Ok(output_paths)
}

fn validate_relative_output_template<'a>(
    interpolated: &'a str,
    step_id: &str,
    template: &str,
) -> Result<&'a Path, OrchestratorError> {
    let relative = Path::new(interpolated);
    if relative.is_absolute() {
        return Err(OrchestratorError::OutputPathValidation {
            step_id: step_id.to_string(),
            template: template.to_string(),
            reason: "output path template must be relative".to_string(),
        });
    }

    let mut has_normal = false;
    for component in relative.components() {
        match component {
            Component::Normal(_) => has_normal = true,
            Component::CurDir | Component::ParentDir => {
                return Err(OrchestratorError::OutputPathValidation {
                    step_id: step_id.to_string(),
                    template: template.to_string(),
                    reason: "non-canonical relative segments (`.` or `..`) are not allowed"
                        .to_string(),
                })
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(OrchestratorError::OutputPathValidation {
                    step_id: step_id.to_string(),
                    template: template.to_string(),
                    reason: "absolute-style segments are not allowed".to_string(),
                })
            }
        }
    }

    if !has_normal {
        return Err(OrchestratorError::OutputPathValidation {
            step_id: step_id.to_string(),
            template: template.to_string(),
            reason: "output template must resolve to a file path".to_string(),
        });
    }

    Ok(relative)
}
