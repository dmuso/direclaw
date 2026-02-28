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

#[test]
fn engineering_feature_delivery_template_includes_task_decomposition_and_loop_gate() {
    let config =
        initial_orchestrator_config("eng", "anthropic", "sonnet", WorkflowTemplate::Engineering);
    let workflow = config
        .workflows
        .iter()
        .find(|workflow| workflow.id == "feature_delivery")
        .expect("feature_delivery workflow should exist");

    let step_ids = workflow
        .steps
        .iter()
        .map(|step| step.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        step_ids,
        vec![
            "plan",
            "plan_review",
            "task_decompose",
            "implement",
            "review",
            "task_loop_gate",
            "done"
        ]
    );

    let task_decompose = workflow
        .steps
        .iter()
        .find(|step| step.id == "task_decompose")
        .expect("task_decompose step should exist");
    assert!(task_decompose
        .outputs
        .iter()
        .any(|key| key.as_str() == "task_list"));

    let plan_review = workflow
        .steps
        .iter()
        .find(|step| step.id == "plan_review")
        .expect("plan_review step should exist");
    assert_eq!(plan_review.on_approve.as_deref(), Some("task_decompose"));
    assert_eq!(plan_review.on_reject.as_deref(), Some("plan"));

    let review = workflow
        .steps
        .iter()
        .find(|step| step.id == "review")
        .expect("review step should exist");
    assert_eq!(review.on_approve.as_deref(), Some("task_loop_gate"));
    assert_eq!(review.on_reject.as_deref(), Some("implement"));

    let loop_gate = workflow
        .steps
        .iter()
        .find(|step| step.id == "task_loop_gate")
        .expect("task_loop_gate step should exist");
    assert_eq!(loop_gate.on_approve.as_deref(), Some("implement"));
    assert_eq!(loop_gate.on_reject.as_deref(), Some("done"));
}
