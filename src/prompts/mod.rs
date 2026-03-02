use crate::config::{ConfigError, OrchestratorConfig, WorkflowStepType};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const PROMPTS_DIR: &str = "prompts";
pub const SELECTOR_PROMPT_REL_PATH: &str = "selector/workflow_selector.prompt.md";
pub const SELECTOR_CONTEXT_REL_PATH: &str = "selector/workflow_selector.context.md";

const SELECTOR_PROMPT_TEMPLATE: &str = include_str!("assets/selector/workflow_selector.prompt.md");
const SELECTOR_CONTEXT_TEMPLATE: &str =
    include_str!("assets/selector/workflow_selector.context.md");
const DEFAULT_TASK_PROMPT_TEMPLATE: &str =
    include_str!("assets/workflow_steps/agent_task.prompt.md");
const DEFAULT_REVIEW_PROMPT_TEMPLATE: &str =
    include_str!("assets/workflow_steps/agent_review.prompt.md");
const DEFAULT_CONTEXT_TEMPLATE: &str = include_str!("assets/workflow_steps/default.context.md");

const MINIMAL_DEFAULT_STEP_1_PROMPT: &str =
    include_str!("assets/workflows/minimal/default/step_1.prompt.md");
const MINIMAL_DEFAULT_STEP_1_CONTEXT: &str =
    include_str!("assets/workflows/minimal/default/step_1.context.md");

const ENG_FEATURE_DELIVERY_PLAN_PROMPT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/plan.prompt.md");
const ENG_FEATURE_DELIVERY_PLAN_CONTEXT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/plan.context.md");
const ENG_FEATURE_DELIVERY_TASK_DECOMPOSE_PROMPT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/task_decompose.prompt.md");
const ENG_FEATURE_DELIVERY_TASK_DECOMPOSE_CONTEXT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/task_decompose.context.md");
const ENG_FEATURE_DELIVERY_PLAN_REVIEW_PROMPT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/plan_review.prompt.md");
const ENG_FEATURE_DELIVERY_PLAN_REVIEW_CONTEXT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/plan_review.context.md");
const ENG_FEATURE_DELIVERY_IMPLEMENT_PROMPT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/implement.prompt.md");
const ENG_FEATURE_DELIVERY_IMPLEMENT_CONTEXT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/implement.context.md");
const ENG_FEATURE_DELIVERY_REVIEW_PROMPT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/review.prompt.md");
const ENG_FEATURE_DELIVERY_REVIEW_CONTEXT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/review.context.md");
const ENG_FEATURE_DELIVERY_TASK_LOOP_GATE_PROMPT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/task_loop_gate.prompt.md");
const ENG_FEATURE_DELIVERY_TASK_LOOP_GATE_CONTEXT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/task_loop_gate.context.md");
const ENG_FEATURE_DELIVERY_DONE_PROMPT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/done.prompt.md");
const ENG_FEATURE_DELIVERY_DONE_CONTEXT: &str =
    include_str!("assets/workflows/engineering/feature_delivery/done.context.md");

const ENG_QUICK_ANSWER_PROMPT: &str =
    include_str!("assets/workflows/engineering/quick_answer/answer.prompt.md");
const ENG_QUICK_ANSWER_CONTEXT: &str =
    include_str!("assets/workflows/engineering/quick_answer/answer.context.md");

const PRODUCT_PRD_RESEARCH_PROMPT: &str =
    include_str!("assets/workflows/product/prd_draft/research.prompt.md");
const PRODUCT_PRD_RESEARCH_CONTEXT: &str =
    include_str!("assets/workflows/product/prd_draft/research.context.md");
const PRODUCT_PRD_DRAFT_PROMPT: &str =
    include_str!("assets/workflows/product/prd_draft/draft.prompt.md");
const PRODUCT_PRD_DRAFT_CONTEXT: &str =
    include_str!("assets/workflows/product/prd_draft/draft.context.md");
const PRODUCT_RELEASE_NOTES_COMPOSE_PROMPT: &str =
    include_str!("assets/workflows/product/release_notes/compose.prompt.md");
const PRODUCT_RELEASE_NOTES_COMPOSE_CONTEXT: &str =
    include_str!("assets/workflows/product/release_notes/compose.context.md");

fn io_create_dir_error(path: &Path, source: std::io::Error) -> ConfigError {
    ConfigError::CreateDir {
        path: path.display().to_string(),
        source,
    }
}

fn io_write_error(path: &Path, source: std::io::Error) -> ConfigError {
    ConfigError::Write {
        path: path.display().to_string(),
        source,
    }
}

pub fn default_step_prompt(step_type: WorkflowStepType) -> &'static str {
    match step_type {
        WorkflowStepType::AgentTask => DEFAULT_TASK_PROMPT_TEMPLATE,
        WorkflowStepType::AgentReview => DEFAULT_REVIEW_PROMPT_TEMPLATE,
    }
}

pub fn default_step_context() -> &'static str {
    DEFAULT_CONTEXT_TEMPLATE
}

pub fn default_selector_prompt() -> &'static str {
    SELECTOR_PROMPT_TEMPLATE
}

pub fn default_selector_context() -> &'static str {
    SELECTOR_CONTEXT_TEMPLATE
}

pub fn default_prompt_rel_path(workflow_id: &str, step_id: &str) -> String {
    format!("workflows/{workflow_id}/{step_id}.prompt.md")
}

pub fn default_context_rel_path(workflow_id: &str, step_id: &str) -> String {
    format!("workflows/{workflow_id}/{step_id}.context.md")
}

fn parse_relative_prompt_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("prompt template path must be non-empty".to_string());
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err("prompt template path must be relative".to_string());
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) => {}
            _ => {
                return Err(
                    "prompt template path must not contain traversal or prefixes".to_string(),
                )
            }
        }
    }
    Ok(candidate.to_path_buf())
}

pub fn resolve_prompt_template_path(prompt_root: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = parse_relative_prompt_path(path)?;
    Ok(prompt_root.join(relative))
}

fn ensure_file(path: &Path, body: &str) -> Result<(), ConfigError> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| io_create_dir_error(parent, source))?;
    }
    fs::write(path, body).map_err(|source| io_write_error(path, source))
}

fn builtin_template_content(
    workflow_id: &str,
    step_id: &str,
) -> Option<(&'static str, &'static str)> {
    match (workflow_id, step_id) {
        ("default", "step_1") => Some((
            MINIMAL_DEFAULT_STEP_1_PROMPT,
            MINIMAL_DEFAULT_STEP_1_CONTEXT,
        )),
        ("feature_delivery", "plan") => Some((
            ENG_FEATURE_DELIVERY_PLAN_PROMPT,
            ENG_FEATURE_DELIVERY_PLAN_CONTEXT,
        )),
        ("feature_delivery", "task_decompose") => Some((
            ENG_FEATURE_DELIVERY_TASK_DECOMPOSE_PROMPT,
            ENG_FEATURE_DELIVERY_TASK_DECOMPOSE_CONTEXT,
        )),
        ("feature_delivery", "plan_review") => Some((
            ENG_FEATURE_DELIVERY_PLAN_REVIEW_PROMPT,
            ENG_FEATURE_DELIVERY_PLAN_REVIEW_CONTEXT,
        )),
        ("feature_delivery", "implement") => Some((
            ENG_FEATURE_DELIVERY_IMPLEMENT_PROMPT,
            ENG_FEATURE_DELIVERY_IMPLEMENT_CONTEXT,
        )),
        ("feature_delivery", "review") => Some((
            ENG_FEATURE_DELIVERY_REVIEW_PROMPT,
            ENG_FEATURE_DELIVERY_REVIEW_CONTEXT,
        )),
        ("feature_delivery", "task_loop_gate") => Some((
            ENG_FEATURE_DELIVERY_TASK_LOOP_GATE_PROMPT,
            ENG_FEATURE_DELIVERY_TASK_LOOP_GATE_CONTEXT,
        )),
        ("feature_delivery", "done") => Some((
            ENG_FEATURE_DELIVERY_DONE_PROMPT,
            ENG_FEATURE_DELIVERY_DONE_CONTEXT,
        )),
        ("quick_answer", "answer") => Some((ENG_QUICK_ANSWER_PROMPT, ENG_QUICK_ANSWER_CONTEXT)),
        ("prd_draft", "research") => {
            Some((PRODUCT_PRD_RESEARCH_PROMPT, PRODUCT_PRD_RESEARCH_CONTEXT))
        }
        ("prd_draft", "draft") => Some((PRODUCT_PRD_DRAFT_PROMPT, PRODUCT_PRD_DRAFT_CONTEXT)),
        ("release_notes", "compose") => Some((
            PRODUCT_RELEASE_NOTES_COMPOSE_PROMPT,
            PRODUCT_RELEASE_NOTES_COMPOSE_CONTEXT,
        )),
        _ => None,
    }
}

pub fn is_prompt_template_reference(path: &str) -> bool {
    let trimmed = path.trim();
    trimmed.ends_with(".md") && !trimmed.contains('\n')
}

pub fn context_path_for_prompt_reference(path: &str) -> String {
    if let Some(base) = path.strip_suffix(".prompt.md") {
        return format!("{base}.context.md");
    }
    format!("{path}.context.md")
}

pub fn ensure_orchestrator_prompt_templates(
    private_workspace: &Path,
    orchestrator: &OrchestratorConfig,
) -> Result<(), ConfigError> {
    let prompt_root = private_workspace.join(PROMPTS_DIR);
    fs::create_dir_all(&prompt_root).map_err(|source| io_create_dir_error(&prompt_root, source))?;

    ensure_file(
        &prompt_root.join(SELECTOR_PROMPT_REL_PATH),
        SELECTOR_PROMPT_TEMPLATE,
    )?;
    ensure_file(
        &prompt_root.join(SELECTOR_CONTEXT_REL_PATH),
        SELECTOR_CONTEXT_TEMPLATE,
    )?;

    for workflow in &orchestrator.workflows {
        for step in &workflow.steps {
            if !is_prompt_template_reference(&step.prompt) {
                continue;
            }
            let prompt_rel = step.prompt.trim();
            let context_rel = context_path_for_prompt_reference(prompt_rel);
            let prompt_path = resolve_prompt_template_path(&prompt_root, prompt_rel)
                .map_err(ConfigError::Orchestrator)?;
            let context_path = resolve_prompt_template_path(&prompt_root, &context_rel)
                .map_err(ConfigError::Orchestrator)?;

            let (prompt_body, context_body) = builtin_template_content(&workflow.id, &step.id)
                .unwrap_or_else(|| (default_step_prompt(step.step_type), default_step_context()));
            ensure_file(&prompt_path, prompt_body)?;
            ensure_file(&context_path, context_body)?;
        }
    }

    Ok(())
}

pub fn render_template_with_placeholders<F>(
    template: &str,
    mut resolve: F,
) -> Result<String, String>
where
    F: FnMut(&str) -> Result<String, String>,
{
    let mut rendered = String::new();
    let mut cursor = template;

    while let Some(start) = cursor.find("{{") {
        rendered.push_str(&cursor[..start]);
        let after_open = &cursor[start + 2..];
        let Some(close_offset) = after_open.find("}}") else {
            return Err("unclosed placeholder in template".to_string());
        };
        let token = after_open[..close_offset].trim();
        if token.is_empty() {
            return Err("empty placeholder in template".to_string());
        }
        rendered.push_str(&resolve(token)?);
        cursor = &after_open[close_offset + 2..];
    }

    rendered.push_str(cursor);
    Ok(rendered)
}

fn has_non_empty_path_segments(raw: &str) -> bool {
    let segments = raw
        .split('.')
        .filter(|segment| !segment.trim().is_empty())
        .collect::<Vec<_>>();
    !segments.is_empty()
}

fn is_supported_step_placeholder(token: &str) -> bool {
    if let Some(path) = token.strip_prefix("inputs.") {
        if path == "memory_bulletin" || path == "memory_bulletin_citations" {
            return false;
        }
        return has_non_empty_path_segments(path);
    }
    if let Some(path) = token.strip_prefix("state.") {
        return has_non_empty_path_segments(path);
    }
    if let Some(path) = token.strip_prefix("steps.") {
        let segments = path.split('.').collect::<Vec<_>>();
        if segments.len() < 3 || segments[1] != "outputs" || segments[0].trim().is_empty() {
            return false;
        }
        return segments[2..]
            .iter()
            .all(|segment| !segment.trim().is_empty());
    }
    if token == "workflow.run_id"
        || token == "workflow.step_id"
        || token == "workflow.attempt"
        || token == "workflow.run_workspace"
        || token == "workflow.output_schema_json"
        || token == "workflow.output_paths_json"
        || token == "workflow.runtime_context_json"
        || token == "workflow.memory_context_bulletin"
        || token == "workflow.memory_context_citations"
        || token == "workflow.memory_context_json"
        || token == "workflow.channel"
        || token == "workflow.channel_profile_id"
        || token == "workflow.conversation_id"
        || token == "workflow.sender_id"
        || token == "workflow.selector_id"
    {
        return true;
    }
    if let Some(path_key) = token.strip_prefix("workflow.output_paths.") {
        return !path_key.trim().is_empty();
    }
    false
}

fn is_supported_selector_placeholder(token: &str) -> bool {
    token == "selector.request_json"
        || token == "selector.result_path"
        || token == "selector.orchestrator_id"
        || token == "selector.agent_id"
        || token == "selector.attempt"
}

fn validate_template_placeholders(template: &str, kind: &str) -> Result<(), String> {
    render_template_with_placeholders(template, |token| {
        let supported = if kind == "selector" {
            is_supported_selector_placeholder(token)
        } else {
            is_supported_step_placeholder(token)
        };
        if supported {
            Ok(String::new())
        } else {
            Err(format!("unsupported {kind} placeholder `{{{{{token}}}}}`"))
        }
    })?;
    Ok(())
}

pub fn validate_orchestrator_prompt_templates(
    private_workspace: &Path,
    orchestrator: &OrchestratorConfig,
) -> Vec<String> {
    let mut issues = Vec::new();
    let prompt_root = private_workspace.join(PROMPTS_DIR);

    let selector_prompt_path = prompt_root.join(SELECTOR_PROMPT_REL_PATH);
    if !selector_prompt_path.is_file() {
        issues.push(format!(
            "selector prompt template missing at {}",
            selector_prompt_path.display()
        ));
    } else if let Ok(body) = fs::read_to_string(&selector_prompt_path) {
        if let Err(err) = validate_template_placeholders(&body, "selector") {
            issues.push(format!(
                "selector prompt template invalid at {}: {err}",
                selector_prompt_path.display()
            ));
        }
    } else {
        issues.push(format!(
            "selector prompt template unreadable at {}",
            selector_prompt_path.display()
        ));
    }

    let selector_context_path = prompt_root.join(SELECTOR_CONTEXT_REL_PATH);
    if !selector_context_path.is_file() {
        issues.push(format!(
            "selector context template missing at {}",
            selector_context_path.display()
        ));
    } else if let Ok(body) = fs::read_to_string(&selector_context_path) {
        if let Err(err) = validate_template_placeholders(&body, "selector") {
            issues.push(format!(
                "selector context template invalid at {}: {err}",
                selector_context_path.display()
            ));
        }
    } else {
        issues.push(format!(
            "selector context template unreadable at {}",
            selector_context_path.display()
        ));
    }

    for workflow in &orchestrator.workflows {
        for step in &workflow.steps {
            if !is_prompt_template_reference(&step.prompt) {
                issues.push(format!(
                    "workflow `{}` step `{}` prompt is inline; expected relative markdown path under prompts/",
                    workflow.id, step.id
                ));
                continue;
            }
            let prompt_path = match resolve_prompt_template_path(&prompt_root, step.prompt.trim()) {
                Ok(path) => path,
                Err(err) => {
                    issues.push(format!(
                        "workflow `{}` step `{}` prompt path `{}` invalid: {err}",
                        workflow.id, step.id, step.prompt
                    ));
                    continue;
                }
            };
            if !prompt_path.is_file() {
                issues.push(format!(
                    "workflow `{}` step `{}` prompt template missing at {}",
                    workflow.id,
                    step.id,
                    prompt_path.display()
                ));
            } else if let Ok(body) = fs::read_to_string(&prompt_path) {
                if let Err(err) = validate_template_placeholders(&body, "step") {
                    issues.push(format!(
                        "workflow `{}` step `{}` prompt template invalid at {}: {err}",
                        workflow.id,
                        step.id,
                        prompt_path.display()
                    ));
                }
            } else {
                issues.push(format!(
                    "workflow `{}` step `{}` prompt template unreadable at {}",
                    workflow.id,
                    step.id,
                    prompt_path.display()
                ));
            }

            let context_rel = context_path_for_prompt_reference(step.prompt.trim());
            let context_path = match resolve_prompt_template_path(&prompt_root, &context_rel) {
                Ok(path) => path,
                Err(err) => {
                    issues.push(format!(
                        "workflow `{}` step `{}` context path `{}` invalid: {err}",
                        workflow.id, step.id, context_rel
                    ));
                    continue;
                }
            };
            if !context_path.is_file() {
                issues.push(format!(
                    "workflow `{}` step `{}` context template missing at {}",
                    workflow.id,
                    step.id,
                    context_path.display()
                ));
            } else if let Ok(body) = fs::read_to_string(&context_path) {
                if let Err(err) = validate_template_placeholders(&body, "step") {
                    issues.push(format!(
                        "workflow `{}` step `{}` context template invalid at {}: {err}",
                        workflow.id,
                        step.id,
                        context_path.display()
                    ));
                }
            } else {
                issues.push(format!(
                    "workflow `{}` step `{}` context template unreadable at {}",
                    workflow.id,
                    step.id,
                    context_path.display()
                ));
            }
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_paths_reject_traversal() {
        let root = Path::new("/tmp/prompts");
        assert!(resolve_prompt_template_path(root, "workflows/default/step_1.prompt.md").is_ok());
        assert!(resolve_prompt_template_path(root, "../bad.md").is_err());
        assert!(resolve_prompt_template_path(root, "/bad.md").is_err());
    }

    #[test]
    fn context_path_is_derived_from_prompt_suffix() {
        assert_eq!(
            context_path_for_prompt_reference("workflows/default/step_1.prompt.md"),
            "workflows/default/step_1.context.md"
        );
    }

    #[test]
    fn step_placeholder_validation_rejects_unknown_tokens() {
        let err = validate_template_placeholders("hello {{workflow.not_supported}}", "step")
            .expect_err("unknown step placeholder should fail");
        assert!(err.contains("unsupported step placeholder"));
    }

    #[test]
    fn step_placeholder_validation_rejects_legacy_memory_placeholders() {
        let err = validate_template_placeholders("{{workflow.memory_bulletin}}", "step")
            .expect_err("legacy memory placeholder should fail");
        assert!(err.contains("unsupported step placeholder"));
    }

    #[test]
    fn step_placeholder_validation_rejects_legacy_inputs_memory_placeholders() {
        let err = validate_template_placeholders("{{inputs.memory_bulletin}}", "step")
            .expect_err("legacy inputs memory placeholder should fail");
        assert!(err.contains("unsupported step placeholder"));
    }

    #[test]
    fn default_selector_prompt_requires_full_result_envelope() {
        let prompt = default_selector_prompt();
        assert!(prompt.contains("selectorId"));
        assert!(prompt.contains("status"));
        assert!(prompt.contains("action"));
        assert!(prompt.contains("selectedWorkflow"));
        assert!(prompt.contains("must include all keys"));
    }

    #[test]
    fn builtin_task_prompts_do_not_allow_blocked_status() {
        let prompts = [
            DEFAULT_TASK_PROMPT_TEMPLATE,
            MINIMAL_DEFAULT_STEP_1_PROMPT,
            ENG_FEATURE_DELIVERY_PLAN_PROMPT,
            ENG_FEATURE_DELIVERY_IMPLEMENT_PROMPT,
            ENG_FEATURE_DELIVERY_DONE_PROMPT,
            ENG_QUICK_ANSWER_PROMPT,
            PRODUCT_PRD_RESEARCH_PROMPT,
            PRODUCT_PRD_DRAFT_PROMPT,
            PRODUCT_RELEASE_NOTES_COMPOSE_PROMPT,
        ];
        for prompt in prompts {
            assert!(
                !prompt.contains("\"blocked\""),
                "prompt must not advertise blocked status: {prompt}"
            );
        }
    }
}
