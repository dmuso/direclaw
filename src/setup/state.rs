use crate::config::{ConfigProviderKind, OrchestratorConfig, SetupDraft};
use crate::workflow::WorkflowTemplate as SetupWorkflowTemplate;

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
        &["gpt-5.2", "gpt-5.3-codex"]
    } else {
        &["sonnet", "opus", "claude-sonnet-4-5", "claude-opus-4-6"]
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
