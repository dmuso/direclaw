use crate::config::{
    AgentConfig, ConfigProviderKind, OrchestratorConfig, OutputKey, PathTemplate, WorkflowConfig,
    WorkflowInputs, WorkflowStepConfig, WorkflowStepType, WorkflowStepWorkspaceMode,
};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowTemplate {
    Minimal,
    Engineering,
    Product,
}

impl WorkflowTemplate {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            WorkflowTemplate::Minimal => "minimal",
            WorkflowTemplate::Engineering => "engineering",
            WorkflowTemplate::Product => "product",
        }
    }
}

pub(crate) fn default_step_prompt(step_type: &str) -> String {
    if step_type == "agent_review" {
        return r#"Return exactly one [workflow_result] JSON envelope.
Execution requirements:
- Evaluate the deliverable against the stated objective, constraints, and quality bar.
- Be explicit, concrete, and evidence-based in summary/feedback.
- Do not emit markdown fences or text outside the envelope.
Required JSON keys:
- decision: "approve" or "reject"
- summary: concise reason for the decision
- feedback: concrete changes needed or verification notes
Decision policy:
- approve only when acceptance criteria are fully met.
- reject when fixes are required; feedback must be actionable.
Output format (JSON only inside envelope):
[workflow_result]{"decision":"approve|reject","summary":"...","feedback":"..."}[/workflow_result]"#
            .to_string();
    }
    r#"Return exactly one [workflow_result] JSON envelope.
Execution requirements:
- Follow the step objective and constraints above.
- Use available workflow context and prior-step outputs when present.
- Do not emit markdown fences or text outside the envelope.
Required JSON keys:
- status: "complete" | "blocked" | "failed"
- summary: concise step summary
- artifact: primary output text for this step
Status policy:
- complete only when the objective is fully satisfied.
- blocked when waiting on missing dependency/permission; include unblock action in summary.
- failed only for unrecoverable errors.
Output format (JSON only inside envelope):
[workflow_result]{"status":"complete|blocked|failed","summary":"...","artifact":"..."}[/workflow_result]"#
        .to_string()
}

pub(crate) fn default_step_scaffold(step_type: &str) -> String {
    let objective = if step_type == "agent_review" {
        "Review the target output against requirements and quality expectations."
    } else {
        "Execute this step objective using available workflow context and workspace artifacts."
    };
    format!("{objective}\n\n{}", default_step_prompt(step_type))
}

pub(crate) fn default_step_output_contract(step_type: &str) -> Vec<OutputKey> {
    if step_type == "agent_review" {
        vec![
            OutputKey::parse("decision").expect("default output key is valid"),
            OutputKey::parse("summary").expect("default output key is valid"),
            OutputKey::parse("feedback").expect("default output key is valid"),
        ]
    } else {
        vec![
            OutputKey::parse("summary").expect("default output key is valid"),
            OutputKey::parse("artifact").expect("default output key is valid"),
        ]
    }
}

pub(crate) fn default_step_output_files(step_type: &str) -> BTreeMap<OutputKey, PathTemplate> {
    if step_type == "agent_review" {
        BTreeMap::from_iter([
            (
                OutputKey::parse_output_file_key("decision").expect("default output key is valid"),
                PathTemplate::parse(
                    "artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-decision.txt",
                )
                .expect("default path template is valid"),
            ),
            (
                OutputKey::parse_output_file_key("summary").expect("default output key is valid"),
                PathTemplate::parse(
                    "artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt",
                )
                .expect("default path template is valid"),
            ),
            (
                OutputKey::parse_output_file_key("feedback")
                    .expect("default output key is valid"),
                PathTemplate::parse(
                    "artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-feedback.txt",
                )
                .expect("default path template is valid"),
            ),
        ])
    } else {
        BTreeMap::from_iter([
            (
                OutputKey::parse_output_file_key("summary").expect("default output key is valid"),
                PathTemplate::parse(
                    "artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}-summary.txt",
                )
                .expect("default path template is valid"),
            ),
            (
                OutputKey::parse_output_file_key("artifact").expect("default output key is valid"),
                PathTemplate::parse(
                    "artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}.txt",
                )
                .expect("default path template is valid"),
            ),
        ])
    }
}

fn workflow_step(id: &str, step_type: &str, agent: &str, prompt: &str) -> WorkflowStepConfig {
    WorkflowStepConfig {
        id: id.to_string(),
        step_type: WorkflowStepType::parse(step_type).expect("default step type is valid"),
        agent: agent.to_string(),
        prompt: format!("{prompt}\n\n{}", default_step_prompt(step_type)),
        workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
        next: None,
        on_approve: None,
        on_reject: None,
        outputs: default_step_output_contract(step_type),
        output_files: default_step_output_files(step_type),
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

pub(crate) fn initial_orchestrator_config(
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
Return one [workflow_result] JSON [/workflow_result] block.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_step_scaffolds_include_workflow_result_contract_and_outputs() {
        let task_prompt = default_step_prompt("agent_task");
        assert!(task_prompt.contains("[workflow_result]"));
        assert!(task_prompt.contains("status"));
        assert!(task_prompt.contains("summary"));
        assert!(task_prompt.contains("artifact"));
        let task_scaffold = default_step_scaffold("agent_task");
        assert!(task_scaffold.contains("Execute this step objective"));
        assert!(!default_step_output_contract("agent_task").is_empty());
        assert!(!default_step_output_files("agent_task").is_empty());
    }

    #[test]
    fn review_step_scaffold_requires_explicit_decision() {
        let review_prompt = default_step_prompt("agent_review");
        assert!(review_prompt.contains("decision"));
        assert!(review_prompt.contains("approve"));
        assert!(review_prompt.contains("reject"));
        assert!(!default_step_output_contract("agent_review").is_empty());
        assert!(!default_step_output_files("agent_review").is_empty());
    }
}
