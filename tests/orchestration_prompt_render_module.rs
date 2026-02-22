use direclaw::config::{
    OutputKey, PathTemplate, WorkflowConfig, WorkflowInputs, WorkflowStepConfig,
    WorkflowStepPromptType, WorkflowStepType, WorkflowStepWorkspaceMode,
};
use direclaw::orchestration::prompt_render::{render_step_prompt, StepSharedWorkspaceContext};
use direclaw::orchestration::run_store::{RunMemoryContext, RunState, WorkflowRunRecord};
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
        memory_context: RunMemoryContext::default(),
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
        final_output_priority: vec![OutputKey::parse("summary").expect("key")],
        limits: None,
    };

    let workflow = WorkflowConfig {
        id: "wf-default".to_string(),
        version: 1,
        description: "default prompt render workflow".to_string(),
        tags: vec!["prompt".parse().expect("tag")],
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
        &[],
        &step.prompt,
        "{{workflow.runtime_context_json}}",
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

#[test]
fn prompt_render_module_injects_memory_context_bundle_with_bounded_citations() {
    let long_bulletin = "goal ".repeat(1500);
    let run = WorkflowRunRecord {
        run_id: "run-mem".to_string(),
        workflow_id: "wf-default".to_string(),
        state: RunState::Running,
        inputs: Map::from_iter([(
            "user_message".to_string(),
            Value::String("plan next steps".to_string()),
        )]),
        memory_context: RunMemoryContext {
            bulletin: long_bulletin,
            citations: vec!["m-1".to_string(), "m-2".to_string(), "m-3".to_string()],
        },
        current_step_id: Some("plan".to_string()),
        current_attempt: Some(1),
        started_at: 10,
        updated_at: 11,
        total_iterations: 1,
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
        prompt: "memory={{workflow.memory_context_bulletin}}\ncitations={{workflow.memory_context_citations}}".to_string(),
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
        final_output_priority: vec![OutputKey::parse("summary").expect("key")],
        limits: None,
    };
    let workflow = WorkflowConfig {
        id: "wf-default".to_string(),
        version: 1,
        description: "workflow".to_string(),
        tags: vec![],
        inputs: WorkflowInputs::default(),
        limits: None,
        steps: vec![step.clone()],
    };
    let output_paths = BTreeMap::from_iter([(
        "summary".to_string(),
        Path::new("/tmp/run-mem/summary.md").to_path_buf(),
    )]);

    let rendered = render_step_prompt(
        &run,
        &workflow,
        &step,
        1,
        Path::new("/tmp/run-mem/workspace"),
        &output_paths,
        &BTreeMap::new(),
        &[],
        &step.prompt,
        "{{workflow.runtime_context_json}}",
    )
    .expect("render prompt");

    assert!(rendered
        .prompt
        .contains("citations=[\"m-1\",\"m-2\",\"m-3\"]"));
    assert!(rendered.context.contains("\"memoryContext\""));
    let context_json: Value = serde_json::from_str(&rendered.context).expect("context json");
    assert!(
        context_json.pointer("/inputs/memory_bulletin").is_none(),
        "runtime context inputs must not duplicate memory bulletin"
    );
    assert!(
        context_json
            .pointer("/inputs/memory_bulletin_citations")
            .is_none(),
        "runtime context inputs must not duplicate memory citations"
    );
    assert!(rendered.prompt.len() < 5000);
}

#[test]
fn prompt_render_module_memory_context_truncation_keeps_complete_ranked_lines() {
    let ranked_lines = (1..=300)
        .map(|i| format!("- ranked memory line {i:03} [m-{i:03}]"))
        .collect::<Vec<_>>()
        .join("\n");
    let run = WorkflowRunRecord {
        run_id: "run-mem-lines".to_string(),
        workflow_id: "wf-default".to_string(),
        state: RunState::Running,
        inputs: Map::from_iter([(
            "user_message".to_string(),
            Value::String("find ranked lines".to_string()),
        )]),
        memory_context: RunMemoryContext {
            bulletin: ranked_lines,
            citations: vec!["m-001".to_string()],
        },
        current_step_id: Some("plan".to_string()),
        current_attempt: Some(1),
        started_at: 10,
        updated_at: 11,
        total_iterations: 1,
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
        prompt: "memory={{workflow.memory_context_bulletin}}".to_string(),
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
        final_output_priority: vec![OutputKey::parse("summary").expect("key")],
        limits: None,
    };
    let workflow = WorkflowConfig {
        id: "wf-default".to_string(),
        version: 1,
        description: "workflow".to_string(),
        tags: vec![],
        inputs: WorkflowInputs::default(),
        limits: None,
        steps: vec![step.clone()],
    };
    let output_paths = BTreeMap::from_iter([(
        "summary".to_string(),
        Path::new("/tmp/run-mem-lines/summary.md").to_path_buf(),
    )]);

    let rendered = render_step_prompt(
        &run,
        &workflow,
        &step,
        1,
        Path::new("/tmp/run-mem-lines/workspace"),
        &output_paths,
        &BTreeMap::new(),
        &[],
        &step.prompt,
        "{{workflow.runtime_context_json}}",
    )
    .expect("render prompt");

    assert!(
        rendered.prompt.contains("- ranked memory line 001 [m-001]"),
        "top ranked line should remain in truncated memory context"
    );
    assert!(
        !rendered.prompt.contains("- ranked memory line 300 [m-300]"),
        "tail lines should be trimmed when memory context is bounded"
    );
    assert!(
        !rendered.prompt.ends_with('['),
        "truncation should not split lines mid-token"
    );
}

#[test]
fn prompt_render_module_context_uses_relative_output_paths_with_shared_root() {
    let run = WorkflowRunRecord {
        run_id: "run-ctx".to_string(),
        workflow_id: "wf-default".to_string(),
        state: RunState::Running,
        inputs: Map::new(),
        memory_context: RunMemoryContext::default(),
        current_step_id: Some("plan".to_string()),
        current_attempt: Some(1),
        started_at: 10,
        updated_at: 11,
        total_iterations: 1,
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
        prompt: "ok".to_string(),
        prompt_type: WorkflowStepPromptType::FileOutput,
        workspace_mode: WorkflowStepWorkspaceMode::RunWorkspace,
        next: None,
        on_approve: None,
        on_reject: None,
        outputs: vec![
            OutputKey::parse("summary").expect("key"),
            OutputKey::parse("artifact").expect("key"),
        ],
        output_files: BTreeMap::from_iter([
            (
                OutputKey::parse_output_file_key("summary").expect("key"),
                PathTemplate::parse("summary.md").expect("template"),
            ),
            (
                OutputKey::parse_output_file_key("artifact").expect("key"),
                PathTemplate::parse("logs/artifact.txt").expect("template"),
            ),
        ]),
        final_output_priority: vec![OutputKey::parse("summary").expect("key")],
        limits: None,
    };
    let workflow = WorkflowConfig {
        id: "wf-default".to_string(),
        version: 1,
        description: "workflow".to_string(),
        tags: vec![],
        inputs: WorkflowInputs::default(),
        limits: None,
        steps: vec![step.clone()],
    };
    let output_paths = BTreeMap::from_iter([
        (
            "summary".to_string(),
            Path::new("/tmp/.direclaw/workflows/runs/run-ctx/steps/plan/attempts/1/summary.md")
                .to_path_buf(),
        ),
        (
            "artifact".to_string(),
            Path::new(
                "/tmp/.direclaw/workflows/runs/run-ctx/steps/plan/attempts/1/logs/artifact.txt",
            )
            .to_path_buf(),
        ),
    ]);

    let rendered = render_step_prompt(
        &run,
        &workflow,
        &step,
        1,
        Path::new("/tmp/.direclaw/workflows/runs/run-ctx/workspace"),
        &output_paths,
        &BTreeMap::new(),
        &[],
        &step.prompt,
        "{{workflow.runtime_context_json}}",
    )
    .expect("render prompt");

    let context_json: Value = serde_json::from_str(&rendered.context).expect("context json");
    assert_eq!(
        context_json.get("outputPathRoot"),
        Some(&Value::String(
            "/tmp/.direclaw/workflows/runs/run-ctx/steps/plan/attempts/1".to_string()
        ))
    );
    assert_eq!(
        context_json.pointer("/outputPaths/summary"),
        Some(&Value::String("summary.md".to_string()))
    );
    assert_eq!(
        context_json.pointer("/outputPaths/artifact"),
        Some(&Value::String("logs/artifact.txt".to_string()))
    );
}

#[test]
fn prompt_render_module_includes_shared_workspaces_in_runtime_context() {
    let run = WorkflowRunRecord {
        run_id: "run-shared".to_string(),
        workflow_id: "wf-default".to_string(),
        state: RunState::Running,
        inputs: Map::new(),
        memory_context: RunMemoryContext::default(),
        current_step_id: Some("plan".to_string()),
        current_attempt: Some(1),
        started_at: 10,
        updated_at: 11,
        total_iterations: 1,
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
        prompt: "ok".to_string(),
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
        final_output_priority: vec![OutputKey::parse("summary").expect("key")],
        limits: None,
    };
    let workflow = WorkflowConfig {
        id: "wf-default".to_string(),
        version: 1,
        description: "workflow".to_string(),
        tags: vec![],
        inputs: WorkflowInputs::default(),
        limits: None,
        steps: vec![step.clone()],
    };
    let output_paths = BTreeMap::from_iter([(
        "summary".to_string(),
        Path::new("/tmp/run-shared/summary.md").to_path_buf(),
    )]);
    let shared_workspaces = vec![StepSharedWorkspaceContext {
        name: "docs".to_string(),
        path: "./shared/docs".to_string(),
        description: "Use this workspace for reference docs.".to_string(),
    }];

    let rendered = render_step_prompt(
        &run,
        &workflow,
        &step,
        1,
        Path::new("/tmp/run-shared/workspace"),
        &output_paths,
        &BTreeMap::new(),
        &shared_workspaces,
        &step.prompt,
        "{{workflow.runtime_context_json}}",
    )
    .expect("render prompt");

    let context_json: Value = serde_json::from_str(&rendered.context).expect("context json");
    assert_eq!(
        context_json.pointer("/sharedWorkspaces/0/name"),
        Some(&Value::String("docs".to_string()))
    );
    assert_eq!(
        context_json.pointer("/sharedWorkspaces/0/path"),
        Some(&Value::String("./shared/docs".to_string()))
    );
    assert_eq!(
        context_json.pointer("/sharedWorkspaces/0/description"),
        Some(&Value::String(
            "Use this workspace for reference docs.".to_string()
        ))
    );
}
