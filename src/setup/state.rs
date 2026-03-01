use crate::config::{
    ConfigProviderKind, OrchestratorConfig, OutputKey, PathTemplate, SetupDraft, WorkflowInputs,
    WorkflowStepConfig,
};
use crate::templates::orchestrator_templates::WorkflowTemplate as SetupWorkflowTemplate;
use std::collections::BTreeMap;

pub(crate) type SetupState = SetupDraft;

pub(crate) fn default_model_for_provider(provider: &str) -> &'static str {
    if provider == "openai" {
        "gpt-5.3-codex"
    } else {
        "sonnet"
    }
}

const PROVIDER_OPTIONS: [ConfigProviderKind; 2] =
    [ConfigProviderKind::Anthropic, ConfigProviderKind::OpenAi];

pub(crate) fn provider_options() -> &'static [ConfigProviderKind] {
    &PROVIDER_OPTIONS
}

pub(crate) fn model_options_for_provider(provider: ConfigProviderKind) -> &'static [&'static str] {
    if provider == ConfigProviderKind::OpenAi {
        &["gpt-5.3-codex", "gpt-5.3-codex-spark"]
    } else {
        &["sonnet", "opus", "haiku"]
    }
}

#[cfg(test)]
pub(crate) fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
    match kind {
        "orchestrator id" => crate::config::OrchestratorId::parse(value).map(|_| ()),
        "workflow id" => crate::config::WorkflowId::parse(value).map(|_| ()),
        "step id" => crate::config::StepId::parse(value).map(|_| ()),
        "agent id" => crate::config::AgentId::parse(value).map(|_| ()),
        _ => Err(format!("unsupported identifier kind `{kind}`")),
    }
}

pub(crate) fn infer_workflow_template(orchestrator: &OrchestratorConfig) -> SetupWorkflowTemplate {
    if orchestrator.agents.contains_key("planner")
        && orchestrator.agents.contains_key("builder")
        && orchestrator.agents.contains_key("reviewer")
    {
        return SetupWorkflowTemplate::Engineering;
    }
    if orchestrator.agents.contains_key("researcher") && orchestrator.agents.contains_key("writer")
    {
        return SetupWorkflowTemplate::Product;
    }
    SetupWorkflowTemplate::Minimal
}

pub(crate) fn setup_workflow_template_index(template: SetupWorkflowTemplate) -> usize {
    match template {
        SetupWorkflowTemplate::Minimal => 0,
        SetupWorkflowTemplate::Engineering => 1,
        SetupWorkflowTemplate::Product => 2,
    }
}

pub(crate) fn workflow_template_from_index(index: usize) -> SetupWorkflowTemplate {
    match index {
        0 => SetupWorkflowTemplate::Minimal,
        1 => SetupWorkflowTemplate::Engineering,
        _ => SetupWorkflowTemplate::Product,
    }
}

pub(crate) fn workflow_template_options() -> Vec<String> {
    vec![
        "minimal: default agent + default workflow (single-step baseline)".to_string(),
        "engineering: planner/builder/reviewer + feature_delivery, quick_answer".to_string(),
        "product: researcher/writer + prd_draft, release_notes".to_string(),
    ]
}

pub(crate) fn workflow_inputs_as_csv(inputs: &WorkflowInputs) -> String {
    let parts: Vec<String> = inputs
        .as_slice()
        .iter()
        .map(|key| key.as_str().to_string())
        .collect();
    if parts.is_empty() {
        "<none>".to_string()
    } else {
        parts.join(",")
    }
}

pub(crate) fn parse_csv_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

pub(crate) fn output_files_as_csv(output_files: &BTreeMap<OutputKey, PathTemplate>) -> String {
    if output_files.is_empty() {
        return "<none>".to_string();
    }
    output_files
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn parse_output_files(raw: &str) -> Result<BTreeMap<OutputKey, PathTemplate>, String> {
    let mut output_files = BTreeMap::new();
    for entry in parse_csv_values(raw) {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| "output_files must use key=path entries".to_string())?;
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            return Err("output_files entries require non-empty key and path".to_string());
        }
        let key = OutputKey::parse_output_file_key(key)?;
        let value = PathTemplate::parse(value)?;
        output_files.insert(key, value);
    }
    Ok(output_files)
}

pub(crate) fn unique_step_id(existing: &[WorkflowStepConfig], base: &str) -> String {
    if !existing.iter().any(|step| step.id == base) {
        return base.to_string();
    }
    let mut idx = 2usize;
    loop {
        let candidate = format!("{base}_{idx}");
        if !existing.iter().any(|step| step.id == candidate) {
            return candidate;
        }
        idx += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkflowStepType;

    #[test]
    fn parse_csv_values_trims_and_filters_empty() {
        assert_eq!(
            parse_csv_values(" alpha, ,beta,gamma  "),
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
    }

    #[test]
    fn parse_output_files_requires_key_value_pairs() {
        let parsed = parse_output_files("result=output/result.md,summary=out/summary.md")
            .expect("valid output files");
        assert_eq!(
            parsed.get("result").map(|template| template.as_str()),
            Some("output/result.md")
        );
        assert_eq!(
            parsed.get("summary").map(|template| template.as_str()),
            Some("out/summary.md")
        );
        assert!(parse_output_files("missing_equals").is_err());
    }

    #[test]
    fn workflow_inputs_as_csv_handles_empty_sequence() {
        assert_eq!(workflow_inputs_as_csv(&WorkflowInputs::default()), "<none>");
    }

    #[test]
    fn unique_step_id_appends_numeric_suffix_when_needed() {
        let existing = vec![
            WorkflowStepConfig {
                id: "step".to_string(),
                step_type: WorkflowStepType::AgentTask,
                agent: "agent".to_string(),
                prompt: "prompt".to_string(),
                prompt_type: crate::config::WorkflowStepPromptType::FileOutput,
                workspace_mode: crate::config::WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                next: None,
                on_approve: None,
                on_reject: None,
                outputs: Vec::new(),
                output_files: BTreeMap::new(),
                final_output_priority: Vec::new(),
                limits: None,
            },
            WorkflowStepConfig {
                id: "step_2".to_string(),
                step_type: WorkflowStepType::AgentTask,
                agent: "agent".to_string(),
                prompt: "prompt".to_string(),
                prompt_type: crate::config::WorkflowStepPromptType::FileOutput,
                workspace_mode: crate::config::WorkflowStepWorkspaceMode::OrchestratorWorkspace,
                next: None,
                on_approve: None,
                on_reject: None,
                outputs: Vec::new(),
                output_files: BTreeMap::new(),
                final_output_priority: Vec::new(),
                limits: None,
            },
        ];

        assert_eq!(unique_step_id(&existing, "step"), "step_3");
        assert_eq!(unique_step_id(&existing, "fresh"), "fresh");
    }
}
