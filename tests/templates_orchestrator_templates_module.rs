use direclaw::templates::orchestrator_templates::{initial_orchestrator_config, WorkflowTemplate};

#[test]
fn orchestrator_templates_module_exposes_minimal_template_builder() {
    let config =
        initial_orchestrator_config("eng", "anthropic", "sonnet", WorkflowTemplate::Minimal);

    assert_eq!(config.id, "eng");
    assert_eq!(config.default_workflow, "default");
    assert_eq!(config.workflows.len(), 1);
    assert_eq!(WorkflowTemplate::Engineering.as_str(), "engineering");
}
