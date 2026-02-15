use crate::config::ConfigError;
#[cfg(test)]
use crate::config::{
    OutputKey, PathTemplate, WorkflowStepConfig, WorkflowStepPromptType, WorkflowStepType,
    WorkflowStepWorkspaceMode,
};
pub use crate::orchestration::function_registry::{FunctionCall, FunctionRegistry};
pub use crate::orchestration::output_contract::{
    evaluate_step_result, interpolate_output_template, parse_review_decision,
    parse_workflow_result_envelope, resolve_step_output_paths, StepEvaluation,
};
pub use crate::orchestration::prompt_render::{render_step_prompt, StepPromptRender};
pub use crate::orchestration::routing::{
    process_queued_message, process_queued_message_with_runner_binaries, resolve_status_run_id,
    StatusResolutionInput,
};
pub use crate::orchestration::run_store::{
    ProgressSnapshot, RunState, SelectorStartedRunMetadata, StepAttemptRecord, WorkflowRunRecord,
    WorkflowRunStore,
};
pub use crate::orchestration::selector::{
    parse_and_validate_selector_result, resolve_orchestrator_id, resolve_selector_with_retries,
    run_selector_attempt_with_provider, FunctionArgSchema, FunctionArgType, FunctionSchema,
    SelectionResolution, SelectorAction, SelectorRequest, SelectorResult, SelectorStatus,
};
pub use crate::orchestration::selector_artifacts::SelectorArtifactStore;
pub use crate::orchestration::transitions::{
    route_selector_action, RouteContext, RoutedSelectorAction,
};
pub use crate::orchestration::workflow_engine::{
    enforce_execution_safety, is_retryable_step_error, resolve_execution_safety_limits,
    resolve_next_step_pointer, resolve_runner_binaries, ExecutionSafetyLimits, NextStepPointer,
    WorkflowEngine,
};
pub use crate::orchestration::workspace_access::{
    enforce_workspace_access, resolve_agent_workspace_root, resolve_workspace_access_context,
    verify_orchestrator_workspace_access, WorkspaceAccessContext,
};
#[cfg(test)]
use serde_json::{Map, Value};
#[cfg(test)]
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("channel message `{message_id}` is missing `channelProfileId`")]
    MissingChannelProfileId { message_id: String },
    #[error("unknown channel profile `{channel_profile_id}`")]
    UnknownChannelProfileId { channel_profile_id: String },
    #[error("selector result is not valid json: {0}")]
    SelectorJson(String),
    #[error("selector validation failed: {0}")]
    SelectorValidation(String),
    #[error("unknown function id `{function_id}`")]
    UnknownFunction { function_id: String },
    #[error("missing required function argument `{arg}`")]
    MissingFunctionArg { arg: String },
    #[error("unknown function argument `{arg}` for `{function_id}`")]
    UnknownFunctionArg { function_id: String, arg: String },
    #[error("invalid argument type for `{function_id}.{arg}`; expected {expected}")]
    InvalidFunctionArgType {
        function_id: String,
        arg: String,
        expected: String,
    },
    #[error("workflow run `{run_id}` not found")]
    UnknownRunId { run_id: String },
    #[error("workflow run state transition `{from}` -> `{to}` is invalid")]
    InvalidRunTransition { from: RunState, to: RunState },
    #[error("workflow result envelope parse failed: {0}")]
    WorkflowEnvelope(String),
    #[error("workflow review decision must be `approve` or `reject`, got `{0}`")]
    InvalidReviewDecision(String),
    #[error("step prompt render failed for step `{step_id}`: {reason}")]
    StepPromptRender { step_id: String, reason: String },
    #[error("step execution failed for step `{step_id}`: {reason}")]
    StepExecution { step_id: String, reason: String },
    #[error("workflow execution exceeded max total iterations ({max_total_iterations})")]
    MaxIterationsExceeded { max_total_iterations: u32 },
    #[error("workflow run timed out after {run_timeout_seconds}s")]
    RunTimeout { run_timeout_seconds: u64 },
    #[error("workflow step timed out after {step_timeout_seconds}s")]
    StepTimeout { step_timeout_seconds: u64 },
    #[error("workspace access denied for orchestrator `{orchestrator_id}` at path `{path}`")]
    WorkspaceAccessDenied {
        orchestrator_id: String,
        path: String,
    },
    #[error("workspace path validation failed for `{path}`: {reason}")]
    WorkspacePathValidation { path: String, reason: String },
    #[error("output path validation failed for step `{step_id}` template `{template}`: {reason}")]
    OutputPathValidation {
        step_id: String,
        template: String,
        reason: String,
    },
    #[error("step `{step_id}` output contract validation failed: {reason}")]
    OutputContractValidation { step_id: String, reason: String },
    #[error("step `{step_id}` transition validation failed: {reason}")]
    TransitionValidation { step_id: String, reason: String },
    #[error("config error: {0}")]
    Config(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("json error at {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

impl From<ConfigError> for OrchestratorError {
    fn from(value: ConfigError) -> Self {
        Self::Config(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::queue::IncomingMessage;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    #[test]
    fn resolve_orchestrator_id_from_channel_profile() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  eng:
    shared_access: []
channel_profiles:
  engineering:
    channel: slack
    orchestrator_id: eng
    slack_app_user_id: U123
    require_mention_in_channels: true
monitoring: {}
channels: {}
"#,
        )
        .expect("settings");

        let inbound = IncomingMessage {
            channel: "slack".to_string(),
            channel_profile_id: Some("engineering".to_string()),
            sender: "dana".to_string(),
            sender_id: "U42".to_string(),
            message: "status?".to_string(),
            timestamp: 1,
            message_id: "m1".to_string(),
            conversation_id: Some("c1".to_string()),
            files: vec![],
            workflow_run_id: None,
            workflow_step_id: None,
        };

        let resolved = resolve_orchestrator_id(&settings, &inbound).expect("resolved");
        assert_eq!(resolved, "eng");
    }

    #[test]
    fn selector_validation_rejects_unknown_function() {
        let request = SelectorRequest {
            selector_id: "sel-1".to_string(),
            channel_profile_id: "engineering".to_string(),
            message_id: "m1".to_string(),
            conversation_id: Some("c1".to_string()),
            user_message: "run command".to_string(),
            available_workflows: vec!["wf".to_string()],
            default_workflow: "wf".to_string(),
            available_functions: vec!["workflow.status".to_string()],
            available_function_schemas: Vec::new(),
        };
        let raw = r#"{
          "selectorId":"sel-1",
          "status":"selected",
          "action":"command_invoke",
          "functionId":"workflow.cancel",
          "functionArgs":{}
        }"#;
        let err = parse_and_validate_selector_result(raw, &request).expect_err("must fail");
        assert!(err.to_string().contains("availableFunctions"));
    }

    #[test]
    fn workflow_result_envelope_parse_and_review_decision() {
        let raw = r#"
ignored
[workflow_result]
{"decision":"approve","feedback":"ok"}
[/workflow_result]
"#;
        let parsed = parse_workflow_result_envelope(raw).expect("parsed");
        let decision = parse_review_decision(&parsed).expect("decision");
        assert!(decision);
    }

    #[test]
    fn run_state_transition_guards_work() {
        assert!(RunState::Queued.can_transition_to(RunState::Running));
        assert!(!RunState::Succeeded.can_transition_to(RunState::Running));
        assert!(!RunState::Failed.can_transition_to(RunState::Running));
    }

    #[test]
    fn workflow_run_record_inputs_round_trip_and_stable_deserialize() {
        let run = WorkflowRunRecord {
            run_id: "run-inputs".to_string(),
            workflow_id: "wf".to_string(),
            state: RunState::Running,
            inputs: Map::from_iter([("ticket".to_string(), Value::String("123".to_string()))]),
            current_step_id: Some("step-1".to_string()),
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
        let encoded = serde_json::to_string(&run).expect("encode");
        let decoded: WorkflowRunRecord = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(
            decoded.inputs.get("ticket"),
            Some(&Value::String("123".to_string()))
        );

        let minimal = r#"{
          "runId":"run-minimal",
          "workflowId":"wf",
          "state":"queued",
          "startedAt":1,
          "updatedAt":1,
          "totalIterations":0
        }"#;
        let decoded_minimal: WorkflowRunRecord = serde_json::from_str(minimal).expect("minimal");
        assert!(decoded_minimal.inputs.is_empty());
    }

    #[test]
    fn workspace_access_context_and_enforcement_allow_private_and_granted_shared_only() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces:
  docs: /tmp/shared/docs
  finance: /tmp/shared/finance
orchestrators:
  alpha:
    shared_access: [docs]
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("settings");

        let context = resolve_workspace_access_context(&settings, "alpha").expect("context");
        assert_eq!(context.shared_workspace_roots.len(), 1);
        assert!(context.shared_workspace_roots.contains_key("docs"));

        enforce_workspace_access(
            &context,
            &[
                PathBuf::from("/tmp/workspace/alpha/agents/worker"),
                PathBuf::from("/tmp/shared/docs/project/readme.md"),
            ],
        )
        .expect("allowed paths");

        let err = enforce_workspace_access(
            &context,
            &[PathBuf::from("/tmp/shared/finance/budget.xlsx")],
        )
        .expect_err("must deny ungranted shared path");
        assert!(err.to_string().contains("workspace access denied"));
    }

    #[test]
    fn output_path_resolution_interpolates_and_blocks_traversal() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join(".direclaw");

        let step = WorkflowStepConfig {
            id: "plan".to_string(),
            step_type: WorkflowStepType::AgentTask,
            agent: "worker".to_string(),
            prompt: "prompt".to_string(),
            prompt_type: WorkflowStepPromptType::FileOutput,
            workspace_mode: WorkflowStepWorkspaceMode::OrchestratorWorkspace,
            next: None,
            on_approve: None,
            on_reject: None,
            outputs: vec![OutputKey::parse("artifact").expect("valid key")],
            output_files: BTreeMap::from_iter([(
                OutputKey::parse_output_file_key("artifact").expect("valid key"),
                PathTemplate::parse(
                    "artifacts/{{workflow.run_id}}/{{workflow.step_id}}-{{workflow.attempt}}.md",
                )
                .expect("valid template"),
            )]),
            limits: None,
        };

        let resolved =
            resolve_step_output_paths(&state_root, "run-123", &step, 2).expect("resolved paths");
        let artifact = resolved.get("artifact").expect("artifact path");
        assert!(artifact
            .starts_with(state_root.join("workflows/runs/run-123/steps/plan/attempts/2/outputs")));
        assert!(artifact
            .display()
            .to_string()
            .ends_with("artifacts/run-123/plan-2.md"));

        let bad_step = WorkflowStepConfig {
            output_files: BTreeMap::from_iter([(
                OutputKey::parse_output_file_key("artifact").expect("valid key"),
                PathTemplate::parse("../escape.md").expect("valid template"),
            )]),
            ..step
        };
        let err =
            resolve_step_output_paths(&state_root, "run-123", &bad_step, 1).expect_err("blocked");
        assert!(err.to_string().contains("output path validation failed"));
    }

    #[test]
    fn function_registry_exposes_machine_readable_schemas_for_v1_scope() {
        let expected_ids = vec![
            "daemon.start",
            "daemon.stop",
            "daemon.restart",
            "daemon.status",
            "daemon.logs",
            "daemon.setup",
            "daemon.send",
            "channels.reset",
            "channels.slack_sync",
            "provider.show",
            "provider.set",
            "model.show",
            "model.set",
            "agent.list",
            "agent.add",
            "agent.show",
            "agent.remove",
            "agent.reset",
            "orchestrator.list",
            "orchestrator.add",
            "orchestrator.show",
            "orchestrator.remove",
            "orchestrator.set_private_workspace",
            "orchestrator.grant_shared_access",
            "orchestrator.revoke_shared_access",
            "orchestrator.set_selector_agent",
            "orchestrator.set_default_workflow",
            "orchestrator.set_selection_max_retries",
            "workflow.list",
            "workflow.show",
            "workflow.add",
            "workflow.remove",
            "workflow.run",
            "workflow.status",
            "workflow.progress",
            "workflow.cancel",
            "channel_profile.list",
            "channel_profile.add",
            "channel_profile.show",
            "channel_profile.remove",
            "channel_profile.set_orchestrator",
            "update.check",
            "update.apply",
            "daemon.attach",
        ];
        let registry = FunctionRegistry::new(expected_ids.iter().map(|id| id.to_string()));
        let schemas = registry.available_function_schemas();
        assert_eq!(schemas.len(), expected_ids.len());
        for expected in &expected_ids {
            assert!(
                schemas.iter().any(|f| &f.function_id == expected),
                "missing function schema for {expected}"
            );
        }
        assert!(schemas
            .iter()
            .any(|f| f.function_id == "workflow.progress" && f.read_only));
        assert!(schemas.iter().any(|f| {
            f.function_id == "workflow.cancel" && !f.read_only && f.args.contains_key("runId")
        }));
        assert!(schemas.iter().any(
            |f| f.function_id == "orchestrator.set_selection_max_retries"
                && f.args.contains_key("count")
        ));
    }

    #[test]
    fn function_registry_rejects_unknown_and_invalid_args() {
        let registry = FunctionRegistry::new(vec!["workflow.status".to_string()]);
        let unknown_arg = FunctionCall {
            function_id: "workflow.status".to_string(),
            args: Map::from_iter([("extra".to_string(), Value::String("x".to_string()))]),
        };
        let err = registry.invoke(&unknown_arg).expect_err("unknown arg");
        assert!(err.to_string().contains("unknown function argument"));

        let invalid_type = FunctionCall {
            function_id: "workflow.status".to_string(),
            args: Map::from_iter([("runId".to_string(), Value::Bool(true))]),
        };
        let err = registry.invoke(&invalid_type).expect_err("invalid type");
        assert!(err.to_string().contains("invalid argument type"));
    }

    #[test]
    fn workflow_status_and_progress_commands_are_read_only() {
        let temp = tempdir().expect("tempdir");
        let store = WorkflowRunStore::new(temp.path());
        let run_id = "run-readonly";
        let mut run = store.create_run(run_id, "wf", 10).expect("create run");
        store
            .transition_state(
                &mut run,
                RunState::Running,
                11,
                "running",
                false,
                "continue",
            )
            .expect("running");
        let before = store.load_run(run_id).expect("before");

        let registry = FunctionRegistry::with_run_store(
            vec![
                "workflow.status".to_string(),
                "workflow.progress".to_string(),
            ],
            store.clone(),
        );
        registry
            .invoke(&FunctionCall {
                function_id: "workflow.status".to_string(),
                args: Map::from_iter([("runId".to_string(), Value::String(run_id.to_string()))]),
            })
            .expect("status call");
        registry
            .invoke(&FunctionCall {
                function_id: "workflow.progress".to_string(),
                args: Map::from_iter([("runId".to_string(), Value::String(run_id.to_string()))]),
            })
            .expect("progress call");

        let after = store.load_run(run_id).expect("after");
        assert_eq!(before.updated_at, after.updated_at);
        assert_eq!(before.state, after.state);
        assert_eq!(before.current_step_id, after.current_step_id);
        assert_eq!(before.current_attempt, after.current_attempt);
    }
}
