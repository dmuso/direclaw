use crate::config::{
    AgentConfig, ConfigProviderKind, OrchestratorConfig, WorkflowConfig, WorkflowInputs,
    WorkflowStepConfig, WorkflowStepPromptType, WorkflowStepType, WorkflowStepWorkspaceMode,
    WorkflowTag,
};
use crate::templates::workflow_step_defaults::{
    default_step_output_contract, default_step_output_files, default_step_output_priority,
    default_step_prompt,
};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowTemplate {
    Minimal,
    Engineering,
    Product,
}

impl WorkflowTemplate {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkflowTemplate::Minimal => "minimal",
            WorkflowTemplate::Engineering => "engineering",
            WorkflowTemplate::Product => "product",
        }
    }
}

fn workflow_step(id: &str, step_type: &str, agent: &str, prompt: &str) -> WorkflowStepConfig {
    WorkflowStepConfig {
        id: id.to_string(),
        step_type: WorkflowStepType::parse(step_type).expect("default step type is valid"),
        agent: agent.to_string(),
        prompt: format!("{prompt}\n\n{}", default_step_prompt(step_type)),
        prompt_type: WorkflowStepPromptType::FileOutput,
        workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
        next: None,
        on_approve: None,
        on_reject: None,
        outputs: default_step_output_contract(step_type),
        output_files: default_step_output_files(step_type),
        final_output_priority: default_step_output_priority(step_type),
        limits: None,
    }
}

fn agent_config(
    provider: &str,
    model: &str,
    private_workspace: &str,
    can_orchestrate_workflows: bool,
) -> AgentConfig {
    AgentConfig {
        provider: ConfigProviderKind::parse(provider).expect("default provider is valid"),
        model: model.to_string(),
        private_workspace: Some(Path::new(private_workspace).to_path_buf()),
        can_orchestrate_workflows,
        shared_access: Vec::new(),
    }
}

fn workflow_tag(value: &str) -> WorkflowTag {
    WorkflowTag::parse(value).expect("default workflow tag is valid")
}

pub fn initial_orchestrator_config(
    id: &str,
    provider: &str,
    model: &str,
    workflow_template: WorkflowTemplate,
) -> OrchestratorConfig {
    let selector = "default".to_string();
    let mut agents = BTreeMap::from_iter([(
        selector.clone(),
        agent_config(provider, model, "agents/default", true),
    )]);
    let (default_workflow, workflows) = match workflow_template {
        WorkflowTemplate::Minimal => {
            let workflow_id = "default".to_string();
            let steps = vec![workflow_step(
                "step_1",
                "agent_task",
                &selector,
                "Execute the user request end-to-end with clear, actionable output.
If requirements are ambiguous, choose reasonable assumptions and state them in summary.
Required outputs schema:
{{workflow.output_schema_json}}
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
artifact -> {{workflow.output_paths.artifact}}",
            )];
            (
                workflow_id.clone(),
                vec![WorkflowConfig {
                    id: workflow_id,
                    version: 1,
                    description: "General purpose workflow for direct requests".to_string(),
                    tags: vec![workflow_tag("default")],
                    inputs: WorkflowInputs::default(),
                    limits: None,
                    steps,
                }],
            )
        }
        WorkflowTemplate::Engineering => {
            agents.insert(
                "planner".to_string(),
                agent_config(provider, model, "agents/planner", false),
            );
            agents.insert(
                "builder".to_string(),
                agent_config(provider, model, "agents/builder", false),
            );
            agents.insert(
                "reviewer".to_string(),
                agent_config(provider, model, "agents/reviewer", false),
            );

            let mut review = workflow_step(
                "review",
                "agent_review",
                "reviewer",
                "Review implementation quality, correctness, and test impact.
Approve only when the work is production-ready.
Write outputs exactly to:
decision -> {{workflow.output_paths.decision}}
summary -> {{workflow.output_paths.summary}}
feedback -> {{workflow.output_paths.feedback}}",
            );
            review.on_approve = Some("done".to_string());
            review.on_reject = Some("implement".to_string());

            let mut implement = workflow_step(
                "implement",
                "agent_task",
                "builder",
                "Implement the approved plan with production-safe changes.
Include changed files and validation results in artifact.
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
artifact -> {{workflow.output_paths.artifact}}",
            );
            implement.next = Some("review".to_string());

            (
                "feature_delivery".to_string(),
                vec![
                    WorkflowConfig {
                        id: "feature_delivery".to_string(),
                        version: 1,
                        description:
                            "Plan, implement, and review engineering work before final summary"
                                .to_string(),
                        tags: vec![
                            workflow_tag("engineering"),
                            workflow_tag("implementation"),
                            workflow_tag("review"),
                        ],
                        inputs: WorkflowInputs::default(),
                        limits: None,
                        steps: vec![
                            {
                                let mut plan = workflow_step(
                                    "plan",
                                    "agent_task",
                                    "planner",
                                    "Draft an implementation plan with risks, sequencing, and test strategy.
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
artifact -> {{workflow.output_paths.artifact}}",
                                );
                                plan.next = Some("implement".to_string());
                                plan
                            },
                            implement,
                            review,
                            workflow_step(
                                "done",
                                "agent_task",
                                "planner",
                                "Summarize final outcome, residual risks, and concrete follow-up actions.
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
artifact -> {{workflow.output_paths.artifact}}",
                            ),
                        ],
                    },
                    WorkflowConfig {
                        id: "quick_answer".to_string(),
                        version: 1,
                        description: "Short direct engineering responses without multi-step review"
                            .to_string(),
                        tags: vec![
                            workflow_tag("engineering"),
                            workflow_tag("quick"),
                            workflow_tag("answer"),
                        ],
                        inputs: WorkflowInputs::default(),
                        limits: None,
                        steps: vec![workflow_step(
                            "answer",
                            "agent_task",
                            "planner",
                            "Answer the user request directly with correct, concise guidance.
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
artifact -> {{workflow.output_paths.artifact}}",
                        )],
                    },
                ],
            )
        }
        WorkflowTemplate::Product => {
            agents.insert(
                "researcher".to_string(),
                agent_config(provider, model, "agents/researcher", false),
            );
            agents.insert(
                "writer".to_string(),
                agent_config(provider, model, "agents/writer", false),
            );

            (
                "prd_draft".to_string(),
                vec![
                    WorkflowConfig {
                        id: "prd_draft".to_string(),
                        version: 1,
                        description: "Research and draft a product requirements document".to_string(),
                        tags: vec![
                            workflow_tag("product"),
                            workflow_tag("prd"),
                            workflow_tag("research"),
                        ],
                        inputs: WorkflowInputs::default(),
                        limits: None,
                        steps: vec![
                            {
                                let mut research = workflow_step(
                                    "research",
                                    "agent_task",
                                    "researcher",
                                    "Extract product constraints, user goals, assumptions, and open questions.
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
artifact -> {{workflow.output_paths.artifact}}",
                                );
                                research.next = Some("draft".to_string());
                                research
                            },
                            workflow_step(
                                "draft",
                                "agent_task",
                                "writer",
                                "Write a concise PRD with problem, goals, scope, milestones, and risks.
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
artifact -> {{workflow.output_paths.artifact}}",
                            ),
                        ],
                    },
                    WorkflowConfig {
                        id: "release_notes".to_string(),
                        version: 1,
                        description:
                            "Compose customer-facing release notes grouped by impact".to_string(),
                        tags: vec![
                            workflow_tag("product"),
                            workflow_tag("release"),
                            workflow_tag("notes"),
                        ],
                        inputs: WorkflowInputs::default(),
                        limits: None,
                        steps: vec![workflow_step(
                            "compose",
                            "agent_task",
                            "writer",
                            "Write release notes grouped by user impact, fixes, and breaking changes.
Write outputs exactly to:
summary -> {{workflow.output_paths.summary}}
artifact -> {{workflow.output_paths.artifact}}",
                        )],
                    },
                ],
            )
        }
    };

    OrchestratorConfig {
        id: id.to_string(),
        selector_agent: selector,
        default_workflow,
        selection_max_retries: 1,
        selector_timeout_seconds: 30,
        agents,
        workflows,
        workflow_orchestration: None,
    }
}
