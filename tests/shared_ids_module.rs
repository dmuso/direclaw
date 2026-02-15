use direclaw::shared::ids::{AgentId, OrchestratorId, StepId, WorkflowId};

#[test]
fn shared_ids_module_parses_domain_identifiers() {
    assert_eq!(OrchestratorId::parse("main").expect("id").as_str(), "main");
    assert_eq!(
        WorkflowId::parse("workflow_1").expect("id").as_str(),
        "workflow_1"
    );
    assert_eq!(StepId::parse("step-1").expect("id").as_str(), "step-1");
    assert_eq!(AgentId::parse("router").expect("id").as_str(), "router");

    assert!(OrchestratorId::parse("main dev").is_err());
    assert!(WorkflowId::parse("").is_err());
    assert!(StepId::parse("step!").is_err());
    assert!(AgentId::parse("agent/id").is_err());
}
