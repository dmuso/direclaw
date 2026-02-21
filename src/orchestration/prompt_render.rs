use crate::config::{WorkflowConfig, WorkflowStepConfig};
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::run_store::WorkflowRunRecord;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MEMORY_CONTEXT_MAX_BULLETIN_CHARS: usize = 4_000;
const MEMORY_CONTEXT_MAX_CITATIONS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepPromptRender {
    pub prompt: String,
    pub context: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryContextBundle {
    bulletin: String,
    citations: Vec<String>,
    truncated: bool,
}

fn build_memory_context_bundle(inputs: &Map<String, Value>) -> MemoryContextBundle {
    let mut truncated = false;
    let mut bulletin = inputs
        .get("memory_bulletin")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if bulletin.chars().count() > MEMORY_CONTEXT_MAX_BULLETIN_CHARS {
        let (value, was_truncated) =
            truncate_memory_bulletin_by_lines(&bulletin, MEMORY_CONTEXT_MAX_BULLETIN_CHARS);
        bulletin = value;
        truncated = was_truncated;
    }
    if bulletin.chars().count() > MEMORY_CONTEXT_MAX_BULLETIN_CHARS {
        bulletin = bulletin
            .chars()
            .take(MEMORY_CONTEXT_MAX_BULLETIN_CHARS)
            .collect();
        truncated = true;
    }

    let mut citations = inputs
        .get("memory_bulletin_citations")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if citations.len() > MEMORY_CONTEXT_MAX_CITATIONS {
        citations.truncate(MEMORY_CONTEXT_MAX_CITATIONS);
        truncated = true;
    }

    MemoryContextBundle {
        bulletin,
        citations,
        truncated,
    }
}

fn truncate_memory_bulletin_by_lines(input: &str, max_chars: usize) -> (String, bool) {
    if input.chars().count() <= max_chars {
        return (input.to_string(), false);
    }

    let mut out = String::new();
    let mut used = 0usize;
    let mut truncated = false;
    for line in input.lines() {
        let line_chars = line.chars().count();
        let separator = if out.is_empty() { 0 } else { 1 };
        if used + separator + line_chars > max_chars {
            if out.is_empty() {
                let hard = line.chars().take(max_chars).collect::<String>();
                return (hard, true);
            }
            truncated = true;
            break;
        }
        if separator == 1 {
            out.push('\n');
            used += 1;
        }
        out.push_str(line);
        used += line_chars;
    }

    (out, truncated || used < input.chars().count())
}

fn resolve_json_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        let object = current.as_object()?;
        current = object.get(*segment)?;
    }
    Some(current)
}

fn value_to_rendered_text(value: &Value) -> Result<String, OrchestratorError> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    serde_json::to_string(value).map_err(|err| {
        OrchestratorError::SelectorJson(format!("failed to render placeholder value: {err}"))
    })
}

fn render_template_with_placeholders<F>(
    template: &str,
    mut resolve: F,
) -> Result<String, OrchestratorError>
where
    F: FnMut(&str) -> Result<String, OrchestratorError>,
{
    let mut rendered = String::new();
    let mut cursor = template;

    while let Some(start) = cursor.find("{{") {
        rendered.push_str(&cursor[..start]);
        let after_open = &cursor[start + 2..];
        let Some(close_offset) = after_open.find("}}") else {
            return Err(OrchestratorError::SelectorValidation(
                "unclosed placeholder in template".to_string(),
            ));
        };
        let token = after_open[..close_offset].trim();
        if token.is_empty() {
            return Err(OrchestratorError::SelectorValidation(
                "empty placeholder in template".to_string(),
            ));
        }
        rendered.push_str(&resolve(token)?);
        cursor = &after_open[close_offset + 2..];
    }

    rendered.push_str(cursor);
    Ok(rendered)
}

fn shared_output_root_and_paths(
    output_paths: &BTreeMap<String, PathBuf>,
) -> (Option<PathBuf>, BTreeMap<String, String>) {
    let mut shared_root: Option<PathBuf> = None;
    for path in output_paths.values() {
        let Some(parent) = path.parent() else {
            shared_root = None;
            break;
        };
        match &mut shared_root {
            None => shared_root = Some(parent.to_path_buf()),
            Some(root) => {
                while !parent.starts_with(&*root) {
                    if !root.pop() {
                        shared_root = None;
                        break;
                    }
                }
            }
        }
        if shared_root.is_none() {
            break;
        }
    }

    let rendered_paths = output_paths
        .iter()
        .map(|(key, path)| {
            let rendered = shared_root
                .as_ref()
                .and_then(|root| path.strip_prefix(root).ok())
                .map(|relative| relative.display().to_string())
                .unwrap_or_else(|| path.display().to_string());
            (key.clone(), rendered)
        })
        .collect::<BTreeMap<_, _>>();

    (shared_root, rendered_paths)
}

pub fn render_step_prompt(
    run: &WorkflowRunRecord,
    workflow: &WorkflowConfig,
    step: &WorkflowStepConfig,
    attempt: u32,
    run_workspace: &Path,
    output_paths: &BTreeMap<String, PathBuf>,
    step_outputs: &BTreeMap<String, Map<String, Value>>,
) -> Result<StepPromptRender, OrchestratorError> {
    let memory_context = build_memory_context_bundle(&run.inputs);
    let input_value = Value::Object(run.inputs.clone());
    let mut state_map = Map::from_iter([
        (
            "run_state".to_string(),
            Value::String(run.state.to_string()),
        ),
        (
            "total_iterations".to_string(),
            Value::from(run.total_iterations),
        ),
        ("started_at".to_string(), Value::from(run.started_at)),
        ("updated_at".to_string(), Value::from(run.updated_at)),
    ]);
    if let Some(step_id) = run.current_step_id.clone() {
        state_map.insert("current_step_id".to_string(), Value::String(step_id));
    }
    if let Some(current_attempt) = run.current_attempt {
        state_map.insert("current_attempt".to_string(), Value::from(current_attempt));
    }
    for (step_id, outputs) in step_outputs {
        for (key, value) in outputs {
            state_map.insert(format!("{step_id}_{key}"), value.clone());
        }
    }
    let state_value = Value::Object(state_map.clone());

    let output_schema_json = serde_json::to_string(
        &step.outputs.clone().into_iter().collect::<Vec<_>>(),
    )
    .map_err(|err| OrchestratorError::StepPromptRender {
        step_id: step.id.clone(),
        reason: format!("failed to render output schema json: {err}"),
    })?;
    let (output_path_root, context_output_paths) = shared_output_root_and_paths(output_paths);

    let output_paths_json = serde_json::to_string_pretty(
        &output_paths
            .iter()
            .map(|(k, v)| (k.clone(), v.display().to_string()))
            .collect::<BTreeMap<_, _>>(),
    )
    .map_err(|err| OrchestratorError::StepPromptRender {
        step_id: step.id.clone(),
        reason: format!("failed to render output paths json: {err}"),
    })?;

    let rendered_prompt = render_template_with_placeholders(&step.prompt, |token| {
        if let Some(path) = token.strip_prefix("inputs.") {
            let path_segments = path
                .split('.')
                .filter(|segment| !segment.trim().is_empty())
                .collect::<Vec<_>>();
            let Some(value) = resolve_json_path(&input_value, &path_segments) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing required placeholder `{{{{{token}}}}}`"),
                });
            };
            return value_to_rendered_text(value);
        }

        if let Some(path) = token.strip_prefix("steps.") {
            let mut segments = path.split('.').collect::<Vec<_>>();
            if segments.len() < 3 || segments[1] != "outputs" {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("unsupported placeholder `{{{{{token}}}}}`"),
                });
            }
            let source_step_id = segments.remove(0).to_string();
            let _ = segments.remove(0);
            let Some(outputs) = step_outputs.get(&source_step_id) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing outputs for placeholder `{{{{{token}}}}}`"),
                });
            };
            let output_value = Value::Object(outputs.clone());
            let Some(value) = resolve_json_path(&output_value, &segments) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing output key for placeholder `{{{{{token}}}}}`"),
                });
            };
            return value_to_rendered_text(value);
        }

        if let Some(path) = token.strip_prefix("state.") {
            let segments = path
                .split('.')
                .filter(|segment| !segment.trim().is_empty())
                .collect::<Vec<_>>();
            let Some(value) = resolve_json_path(&state_value, &segments) else {
                return Ok(String::new());
            };
            return value_to_rendered_text(value);
        }

        if token == "workflow.run_id" {
            return Ok(run.run_id.clone());
        }
        if token == "workflow.step_id" {
            return Ok(step.id.clone());
        }
        if token == "workflow.attempt" {
            return Ok(attempt.to_string());
        }
        if token == "workflow.run_workspace" {
            return Ok(run_workspace.display().to_string());
        }
        if token == "workflow.output_schema_json" {
            return Ok(output_schema_json.clone());
        }
        if token == "workflow.output_paths_json" {
            return Ok(output_paths_json.clone());
        }
        if token == "workflow.memory_context_bulletin" {
            return Ok(memory_context.bulletin.clone());
        }
        if token == "workflow.memory_context_citations" {
            return serde_json::to_string(&memory_context.citations).map_err(|err| {
                OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("failed to render memory context citations: {err}"),
                }
            });
        }
        if token == "workflow.memory_context_json" {
            return serde_json::to_string(&Value::Object(Map::from_iter([
                (
                    "bulletin".to_string(),
                    Value::String(memory_context.bulletin.clone()),
                ),
                (
                    "citations".to_string(),
                    Value::Array(
                        memory_context
                            .citations
                            .iter()
                            .map(|value| Value::String(value.clone()))
                            .collect(),
                    ),
                ),
                (
                    "truncated".to_string(),
                    Value::Bool(memory_context.truncated),
                ),
            ])))
            .map_err(|err| OrchestratorError::StepPromptRender {
                step_id: step.id.clone(),
                reason: format!("failed to render memory context json: {err}"),
            });
        }
        if let Some(path_key) = token.strip_prefix("workflow.output_paths.") {
            let Some(path) = output_paths.get(path_key) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing output path for placeholder `{{{{{token}}}}}`"),
                });
            };
            return Ok(path.display().to_string());
        }

        let input_field = match token {
            "workflow.channel" => Some("channel"),
            "workflow.channel_profile_id" => Some("channel_profile_id"),
            "workflow.conversation_id" => Some("conversation_id"),
            "workflow.sender_id" => Some("sender_id"),
            "workflow.selector_id" => Some("selector_id"),
            "workflow.memory_bulletin" => Some("memory_bulletin"),
            "workflow.memory_bulletin_citations" => Some("memory_bulletin_citations"),
            _ => None,
        };
        if let Some(field) = input_field {
            let Some(value) = run.inputs.get(field) else {
                return Err(OrchestratorError::StepPromptRender {
                    step_id: step.id.clone(),
                    reason: format!("missing required placeholder `{{{{{token}}}}}`"),
                });
            };
            return value_to_rendered_text(value);
        }

        Err(OrchestratorError::StepPromptRender {
            step_id: step.id.clone(),
            reason: format!("unsupported placeholder `{{{{{token}}}}}`"),
        })
    })?;

    let context = serde_json::to_string_pretty(&Value::Object(Map::from_iter([
        ("runId".to_string(), Value::String(run.run_id.clone())),
        ("workflowId".to_string(), Value::String(workflow.id.clone())),
        ("stepId".to_string(), Value::String(step.id.clone())),
        ("attempt".to_string(), Value::from(attempt)),
        (
            "runWorkspace".to_string(),
            Value::String(run_workspace.display().to_string()),
        ),
        ("inputs".to_string(), Value::Object(run.inputs.clone())),
        ("state".to_string(), Value::Object(state_map)),
        (
            "availableStepOutputs".to_string(),
            Value::Object(Map::from_iter(step_outputs.iter().map(
                |(step_id, outputs)| (step_id.clone(), Value::Object(outputs.clone())),
            ))),
        ),
        (
            "outputPaths".to_string(),
            Value::Object(Map::from_iter(
                context_output_paths
                    .iter()
                    .map(|(k, path)| (k.clone(), Value::String(path.clone()))),
            )),
        ),
        (
            "outputPathRoot".to_string(),
            output_path_root
                .map(|root| Value::String(root.display().to_string()))
                .unwrap_or(Value::Null),
        ),
        (
            "memoryContext".to_string(),
            Value::Object(Map::from_iter([
                (
                    "bulletin".to_string(),
                    Value::String(memory_context.bulletin.clone()),
                ),
                (
                    "citations".to_string(),
                    Value::Array(
                        memory_context
                            .citations
                            .iter()
                            .map(|value| Value::String(value.clone()))
                            .collect(),
                    ),
                ),
                (
                    "truncated".to_string(),
                    Value::Bool(memory_context.truncated),
                ),
                (
                    "maxBulletinChars".to_string(),
                    Value::from(MEMORY_CONTEXT_MAX_BULLETIN_CHARS as u64),
                ),
            ])),
        ),
    ])))
    .map_err(|err| OrchestratorError::StepPromptRender {
        step_id: step.id.clone(),
        reason: format!("failed to render context artifact: {err}"),
    })?;

    Ok(StepPromptRender {
        prompt: rendered_prompt,
        context,
    })
}
