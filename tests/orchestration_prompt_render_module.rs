use direclaw::config::{
    OutputKey, PathTemplate, WorkflowConfig, WorkflowInputs, WorkflowStepConfig,
    WorkflowStepPromptType, WorkflowStepType, WorkflowStepWorkspaceMode,
};
use direclaw::orchestration::prompt_render::render_step_prompt;
use direclaw::orchestration::run_store::{RunState, WorkflowRunRecord};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

#[test]
fn prompt_render_module_renders_inputs_state_and_output_paths() {
    let run = WorkflowRunRecord {
        run_id: "run-123".to_string(),
        workflow_id: "wf-default".to_string(),
        state: RunState::Running,
        inputs: Map::from_iter([
            (
                "channel_profile_id".to_string(),
                Value::String("eng".to_string()),
            ),
            (
                "user_message".to_string(),
                Value::String("summarize queue status".to_string()),
            ),
        ]),
        current_step_id: Some("plan".to_string()),
        current_attempt: Some(2),
        started_at: 100,
        updated_at: 101,
        total_iterations: 3,
        source_message_id: None,
        selector_id: None,
        selected_workflow: None,
        status_conversation_id: None,
        terminal_reason: None,
    };

    let step = WorkflowStepConfig {
        id: "plan".to_string(),
        step_type: WorkflowStepType::AgentTask,
        agent: "worker".to_string(),
        prompt: "Run {{workflow.run_id}} as {{inputs.user_message}} and write to {{workflow.output_paths.summary}} using state={{state.total_iterations}}".to_string(),
        prompt_type: WorkflowStepPromptType::FileOutput,
        workspace_mode: WorkflowStepWorkspaceMode::RunWorkspace,
        next: None,
        on_approve: None,
        on_reject: None,
        outputs: vec![OutputKey::parse("summary").expect("key")],
        output_files: BTreeMap::from_iter([(
            OutputKey::parse_output_file_key("summary").expect("key"),
            PathTemplate::parse("summary.md").expect("template"),
        )]),
        limits: None,
    };

    let workflow = WorkflowConfig {
        id: "wf-default".to_string(),
        version: 1,
        inputs: WorkflowInputs::default(),
        limits: None,
        steps: vec![step.clone()],
    };

    let output_paths = BTreeMap::from_iter([(
        "summary".to_string(),
        Path::new("/tmp/run-123/summary.md").to_path_buf(),
    )]);
    let step_outputs = BTreeMap::new();

    let rendered = render_step_prompt(
        &run,
        &workflow,
        &step,
        1,
        Path::new("/tmp/run-123/workspace"),
        &output_paths,
        &step_outputs,
    )
    .expect("render prompt");

    assert!(rendered
        .prompt
        .contains("Run run-123 as summarize queue status"));
    assert!(rendered.prompt.contains("summary.md"));
    assert!(rendered.prompt.contains("state=3"));
    assert!(rendered.context.contains("\"workflowId\": \"wf-default\""));
    assert!(rendered.context.contains("\"stepId\": \"plan\""));
}
