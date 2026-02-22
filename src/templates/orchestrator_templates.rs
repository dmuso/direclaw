use crate::config::{
    AgentConfig, ConfigProviderKind, OrchestratorConfig, WorkflowConfig, WorkflowInputs,
    WorkflowStepConfig, WorkflowStepPromptType, WorkflowStepType, WorkflowStepWorkspaceMode,
    WorkflowTag,
};
use crate::prompts::default_prompt_rel_path;
use crate::templates::workflow_step_defaults::{
    default_step_output_contract, default_step_output_files, default_step_output_priority,
};
use std::collections::BTreeMap;

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

fn workflow_step(
    workflow_id: &str,
    id: &str,
    step_type: &str,
    agent: &str,
    prompt_rel_path: &str,
) -> WorkflowStepConfig {
    WorkflowStepConfig {
        id: id.to_string(),
        step_type: WorkflowStepType::parse(step_type).expect("default step type is valid"),
        agent: agent.to_string(),
        prompt: if prompt_rel_path.trim().is_empty() {
            default_prompt_rel_path(workflow_id, id)
        } else {
            prompt_rel_path.to_string()
        },
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

fn agent_config(provider: &str, model: &str, can_orchestrate_workflows: bool) -> AgentConfig {
    AgentConfig {
        provider: ConfigProviderKind::parse(provider).expect("default provider is valid"),
        model: model.to_string(),
        can_orchestrate_workflows,
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
    let mut agents = BTreeMap::from_iter([(selector.clone(), agent_config(provider, model, true))]);
    let (default_workflow, workflows) = match workflow_template {
        WorkflowTemplate::Minimal => {
            let workflow_id = "default".to_string();
            let steps = vec![workflow_step(
                &workflow_id,
                "step_1",
                "agent_task",
                &selector,
                &default_prompt_rel_path("default", "step_1"),
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
            agents.insert("planner".to_string(), agent_config(provider, model, false));
            agents.insert("builder".to_string(), agent_config(provider, model, false));
            agents.insert("reviewer".to_string(), agent_config(provider, model, false));

            let mut review = workflow_step(
                "feature_delivery",
                "review",
                "agent_review",
                "reviewer",
                &default_prompt_rel_path("feature_delivery", "review"),
            );
            review.on_approve = Some("done".to_string());
            review.on_reject = Some("implement".to_string());

            let mut implement = workflow_step(
                "feature_delivery",
                "implement",
                "agent_task",
                "builder",
                &default_prompt_rel_path("feature_delivery", "implement"),
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
                                    "feature_delivery",
                                    "plan",
                                    "agent_task",
                                    "planner",
                                    &default_prompt_rel_path("feature_delivery", "plan"),
                                );
                                plan.next = Some("implement".to_string());
                                plan
                            },
                            implement,
                            review,
                            workflow_step(
                                "feature_delivery",
                                "done",
                                "agent_task",
                                "planner",
                                &default_prompt_rel_path("feature_delivery", "done"),
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
                            "quick_answer",
                            "answer",
                            "agent_task",
                            "planner",
                            &default_prompt_rel_path("quick_answer", "answer"),
                        )],
                    },
                ],
            )
        }
        WorkflowTemplate::Product => {
            agents.insert(
                "researcher".to_string(),
                agent_config(provider, model, false),
            );
            agents.insert("writer".to_string(), agent_config(provider, model, false));

            (
                "prd_draft".to_string(),
                vec![
                    WorkflowConfig {
                        id: "prd_draft".to_string(),
                        version: 1,
                        description: "Research and draft a product requirements document"
                            .to_string(),
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
                                    "prd_draft",
                                    "research",
                                    "agent_task",
                                    "researcher",
                                    &default_prompt_rel_path("prd_draft", "research"),
                                );
                                research.next = Some("draft".to_string());
                                research
                            },
                            workflow_step(
                                "prd_draft",
                                "draft",
                                "agent_task",
                                "writer",
                                &default_prompt_rel_path("prd_draft", "draft"),
                            ),
                        ],
                    },
                    WorkflowConfig {
                        id: "release_notes".to_string(),
                        version: 1,
                        description: "Compose customer-facing release notes grouped by impact"
                            .to_string(),
                        tags: vec![
                            workflow_tag("product"),
                            workflow_tag("release"),
                            workflow_tag("notes"),
                        ],
                        inputs: WorkflowInputs::default(),
                        limits: None,
                        steps: vec![workflow_step(
                            "release_notes",
                            "compose",
                            "agent_task",
                            "writer",
                            &default_prompt_rel_path("release_notes", "compose"),
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
