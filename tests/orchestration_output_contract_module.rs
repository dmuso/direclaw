use direclaw::config::{
    OutputKey, PathTemplate, WorkflowConfig, WorkflowInputs, WorkflowStepConfig,
    WorkflowStepPromptType, WorkflowStepType, WorkflowStepWorkspaceMode,
};
use direclaw::orchestration::output_contract::{
    evaluate_step_result, parse_review_decision, parse_workflow_result_envelope,
    resolve_step_output_paths,
};
use std::collections::BTreeMap;

#[test]
fn output_contract_module_parses_envelope_and_resolves_paths() {
    let step = WorkflowStepConfig {
        id: "plan".to_string(),
        step_type: WorkflowStepType::AgentReview,
        agent: "worker".to_string(),
        prompt: "review".to_string(),
        prompt_type: WorkflowStepPromptType::WorkflowResultEnvelope,
        workspace_mode: WorkflowStepWorkspaceMode::RunWorkspace,
        next: None,
        on_approve: Some("done".to_string()),
        on_reject: Some("plan".to_string()),
        outputs: vec![
            OutputKey::parse("decision").expect("decision key"),
            OutputKey::parse("feedback").expect("feedback key"),
        ],
        output_files: BTreeMap::from_iter([(
            OutputKey::parse_output_file_key("feedback").expect("feedback output key"),
            PathTemplate::parse("artifacts/{{workflow.run_id}}/{{workflow.step_id}}.md")
                .expect("template"),
        )]),
        limits: None,
    };
    let done = WorkflowStepConfig {
        id: "done".to_string(),
        step_type: WorkflowStepType::AgentTask,
        agent: "worker".to_string(),
        prompt: "done".to_string(),
        prompt_type: WorkflowStepPromptType::FileOutput,
        workspace_mode: WorkflowStepWorkspaceMode::RunWorkspace,
        next: None,
        on_approve: None,
        on_reject: None,
        outputs: Vec::new(),
        output_files: BTreeMap::new(),
        limits: None,
    };
    let workflow = WorkflowConfig {
        id: "wf".to_string(),
        version: 1,
        inputs: WorkflowInputs::default(),
        limits: None,
        steps: vec![step.clone(), done],
    };

    let parsed = parse_workflow_result_envelope(
        "[workflow_result]{\"decision\":\"approve\",\"feedback\":\"ok\"}[/workflow_result]",
    )
    .expect("parse envelope");
    assert!(parse_review_decision(&parsed).expect("review decision"));

    let paths =
        resolve_step_output_paths(std::path::Path::new("/tmp/.direclaw"), "run-1", &step, 2)
            .expect("resolve output paths");
    assert!(paths
        .get("feedback")
        .expect("feedback path")
        .display()
        .to_string()
        .ends_with("workflows/runs/run-1/steps/plan/attempts/2/outputs/artifacts/run-1/plan.md"));

    let evaluation = evaluate_step_result(
        &workflow,
        &step,
        "[workflow_result]{\"decision\":\"approve\",\"feedback\":\"ok\"}[/workflow_result]",
        &paths,
    )
    .expect("evaluate");
    assert_eq!(evaluation.next_step_id.as_deref(), Some("done"));
}

#[test]
fn output_contract_module_accepts_prefixed_review_decision_values() {
    let parsed = parse_workflow_result_envelope(
        "[workflow_result]{\"decision\":\"decision: approve\"}[/workflow_result]",
    )
    .expect("parse envelope");
    assert!(parse_review_decision(&parsed).expect("review decision"));
}
