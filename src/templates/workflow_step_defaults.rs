use crate::config::{OutputKey, PathTemplate};
use std::collections::BTreeMap;

pub fn default_step_prompt(step_type: &str) -> String {
    if step_type == "agent_review" {
        return r#"Instructions:
1. Read available context from the provided prompt/context/input files before acting.
2. Execute the step objective and follow the user request when it applies.
3. Follow the additional requirements below.
4. When complete, write structured output values to the exact file paths listed under "Write outputs exactly to". Do not rely on stdout for structured output.
Execution requirements:
- Evaluate the deliverable against the stated objective, constraints, and quality bar.
- Be explicit, concrete, and evidence-based in summary/feedback.
Required structured output keys:
- decision: "approve" or "reject"
- summary: concise reason for the decision
- feedback: concrete changes needed or verification notes
Decision policy:
- approve only when acceptance criteria are fully met.
- reject when fixes are required; feedback must be actionable.
Write outputs exactly to:
- decision -> {{workflow.output_paths.decision}}
- summary -> {{workflow.output_paths.summary}}
- feedback -> {{workflow.output_paths.feedback}}"#
            .to_string();
    }
    r#"Instructions:
1. Read available context from the provided prompt/context/input files before acting.
2. Execute the step objective and follow the user request when it applies.
3. Follow the additional requirements below.
4. When complete, write structured output values to the exact file paths listed under "Write outputs exactly to". Do not rely on stdout for structured output.
Execution requirements:
- Follow the step objective and constraints above.
- Use available workflow context and prior-step outputs when present.
Required structured output keys:
- status: "complete" | "blocked" | "failed"
- summary: concise step summary
- artifact: primary output text for this step
Status policy:
- complete only when the objective is fully satisfied.
- blocked when waiting on missing dependency/permission; include unblock action in summary.
- failed only for unrecoverable errors.
Write outputs exactly to:
- summary -> {{workflow.output_paths.summary}}
- artifact -> {{workflow.output_paths.artifact}}"#
        .to_string()
}

pub fn default_step_scaffold(step_type: &str) -> String {
    let objective = if step_type == "agent_review" {
        "Review the target output against requirements and quality expectations."
    } else {
        "Execute this step objective using available workflow context and workspace artifacts."
    };
    format!("{objective}\n\n{}", default_step_prompt(step_type))
}

pub fn default_step_output_contract(step_type: &str) -> Vec<OutputKey> {
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

pub fn default_step_output_files(step_type: &str) -> BTreeMap<OutputKey, PathTemplate> {
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

pub fn default_step_output_priority(step_type: &str) -> Vec<OutputKey> {
    if step_type == "agent_review" {
        vec![OutputKey::parse("summary").expect("default output key is valid")]
    } else {
        vec![
            OutputKey::parse("artifact").expect("default output key is valid"),
            OutputKey::parse("summary").expect("default output key is valid"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_step_scaffolds_include_file_output_contract_and_outputs() {
        let task_prompt = default_step_prompt("agent_task");
        assert!(task_prompt.contains("When complete, write structured output values"));
        assert!(task_prompt.contains("workflow.output_paths.summary"));
        assert!(task_prompt.contains("status"));
        assert!(task_prompt.contains("summary"));
        assert!(task_prompt.contains("artifact"));
        let task_scaffold = default_step_scaffold("agent_task");
        assert!(task_scaffold.contains("Execute this step objective"));
        assert!(!default_step_output_contract("agent_task").is_empty());
        assert!(!default_step_output_files("agent_task").is_empty());
        assert_eq!(
            default_step_output_priority("agent_task")
                .into_iter()
                .map(|key| key.name)
                .collect::<Vec<_>>(),
            vec!["artifact".to_string(), "summary".to_string()]
        );
    }

    #[test]
    fn review_step_scaffold_requires_explicit_decision() {
        let review_prompt = default_step_prompt("agent_review");
        assert!(review_prompt.contains("decision"));
        assert!(review_prompt.contains("approve"));
        assert!(review_prompt.contains("reject"));
        assert!(!default_step_output_contract("agent_review").is_empty());
        assert!(!default_step_output_files("agent_review").is_empty());
        assert_eq!(
            default_step_output_priority("agent_review")
                .into_iter()
                .map(|key| key.name)
                .collect::<Vec<_>>(),
            vec!["summary".to_string()]
        );
    }
}
