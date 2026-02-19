use crate::config::{OrchestratorConfig, Settings};
use crate::orchestration::diagnostics::{persist_selector_invocation_log, provider_error_log};
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::workspace_access::resolve_agent_workspace_root;
use crate::provider::{
    run_provider, write_file_backed_prompt, ProviderKind, ProviderRequest, RunnerBinaries,
};
use crate::queue::IncomingMessage;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

pub fn resolve_orchestrator_id(
    settings: &Settings,
    inbound: &IncomingMessage,
) -> Result<String, OrchestratorError> {
    let channel_profile_id = inbound
        .channel_profile_id
        .as_ref()
        .filter(|id| !id.trim().is_empty());

    if channel_profile_id.is_none() && inbound.channel == "heartbeat" {
        if let Some(orchestrator_id) = inbound
            .sender
            .strip_prefix("heartbeat:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if settings.orchestrators.contains_key(orchestrator_id) {
                return Ok(orchestrator_id.to_string());
            }
        }
    }
    if channel_profile_id.is_none() && inbound.channel == "scheduler" {
        if let Some(orchestrator_id) = inbound
            .sender
            .strip_prefix("scheduler:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if settings.orchestrators.contains_key(orchestrator_id) {
                return Ok(orchestrator_id.to_string());
            }
        }
    }

    let channel_profile_id =
        channel_profile_id.ok_or_else(|| OrchestratorError::MissingChannelProfileId {
            message_id: inbound.message_id.clone(),
        })?;

    let profile = settings
        .channel_profiles
        .get(channel_profile_id)
        .ok_or_else(|| OrchestratorError::UnknownChannelProfileId {
            channel_profile_id: channel_profile_id.to_string(),
        })?;
    Ok(profile.orchestrator_id.clone())
}

pub fn run_selector_attempt_with_provider(
    state_root: &Path,
    settings: &Settings,
    request: &SelectorRequest,
    orchestrator: &OrchestratorConfig,
    attempt: u32,
    binaries: &RunnerBinaries,
) -> Result<String, String> {
    let selector_agent = orchestrator
        .agents
        .get(&orchestrator.selector_agent)
        .ok_or_else(|| {
            format!(
                "selector agent `{}` missing from orchestrator config",
                orchestrator.selector_agent
            )
        })?;
    let provider = ProviderKind::try_from(selector_agent.provider.as_str())
        .map_err(|err| format!("invalid selector provider: {err}"))?;

    let private_workspace = settings
        .resolve_private_workspace(&orchestrator.id)
        .map_err(|err| err.to_string())?;
    let cwd = resolve_agent_workspace_root(
        &private_workspace,
        &orchestrator.selector_agent,
        selector_agent,
    );
    fs::create_dir_all(&cwd).map_err(|err| err.to_string())?;

    let request_json = serde_json::to_string_pretty(request).map_err(|err| err.to_string())?;
    let selector_result_path = cwd.join("orchestrator/artifacts").join(format!(
        "selector-provider-result-{}_attempt_{attempt}.json",
        request.selector_id
    ));
    if let Some(parent) = selector_result_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let prompt = format!(
        "You are the workflow selector.\nRead this selector request JSON and select the next action.\n{request_json}\n\nDecision policy:\n- Prioritize the user's explicit requested action over surrounding background context.\n- Distinguish contextual setup/background from the actual ask before choosing an action.\n- Use background details only to inform the action, not to override the direct request.\n- Choose from availableWorkflows/defaultWorkflow/availableFunctions exactly as provided.\n- Action `no_response` is allowed only for low-value opportunistic context messages.\n- Never use `no_response` when the inbound context indicates an explicit profile mention.\n\nInstructions:\n1. Read the selector request from the provided files.\n2. Identify the user's requested action separately from contextual setup/background.\n3. Select exactly one supported action and validate any selected workflow/function against the request fields.\n4. Output exactly one structured JSON selector result to this path:\n{}\n5. Do not output structured JSON anywhere else and do not rely on stdout.\nDo not use markdown fences.",
        selector_result_path.display()
    );
    let context = format!(
        "orchestratorId={}\nselectorAgent={}\nattempt={attempt}\nselectorResultPath={}",
        orchestrator.id,
        orchestrator.selector_agent,
        selector_result_path.display()
    );
    let request_id = format!("{}_attempt_{attempt}", request.selector_id);
    let artifacts = write_file_backed_prompt(&cwd, &request_id, &prompt, &context)
        .map_err(|err| err.to_string())?;

    let provider_request = ProviderRequest {
        agent_id: orchestrator.selector_agent.clone(),
        provider,
        model: selector_agent.model.clone(),
        cwd: cwd.clone(),
        message: format!(
            "Read [file: {}] and [file: {}]. Write selector result JSON to: {}",
            artifacts.prompt_file.display(),
            artifacts
                .context_files
                .first()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            selector_result_path.display()
        ),
        prompt_artifacts: artifacts,
        timeout: Duration::from_secs(orchestrator.selector_timeout_seconds),
        reset_requested: false,
        fresh_on_failure: false,
        env_overrides: BTreeMap::new(),
    };

    match run_provider(&provider_request, binaries) {
        Ok(result) => {
            persist_selector_invocation_log(
                state_root,
                &request.selector_id,
                attempt,
                Some(&result.log),
                None,
            );
            fs::read_to_string(&selector_result_path).map_err(|err| {
                format!(
                    "selector did not write result file at {}: {}",
                    selector_result_path.display(),
                    err
                )
            })
        }
        Err(err) => {
            let error_text = err.to_string();
            persist_selector_invocation_log(
                state_root,
                &request.selector_id,
                attempt,
                provider_error_log(&err),
                Some(&error_text),
            );
            Err(error_text)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionArgType {
    String,
    Boolean,
    Integer,
    Object,
}

impl FunctionArgType {
    pub(crate) fn matches(&self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Boolean => value.is_boolean(),
            Self::Integer => value.is_i64() || value.is_u64(),
            Self::Object => value.is_object(),
        }
    }
}

impl std::fmt::Display for FunctionArgType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String => write!(f, "string"),
            Self::Boolean => write!(f, "boolean"),
            Self::Integer => write!(f, "integer"),
            Self::Object => write!(f, "object"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionArgSchema {
    #[serde(rename = "type")]
    pub arg_type: FunctionArgType,
    pub required: bool,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionSchema {
    pub function_id: String,
    pub description: String,
    #[serde(default)]
    pub args: BTreeMap<String, FunctionArgSchema>,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectorRequest {
    pub selector_id: String,
    pub channel_profile_id: String,
    pub message_id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    pub user_message: String,
    #[serde(default)]
    pub memory_bulletin: Option<String>,
    #[serde(default)]
    pub memory_bulletin_citations: Vec<String>,
    pub available_workflows: Vec<String>,
    pub default_workflow: String,
    #[serde(default)]
    pub available_functions: Vec<String>,
    #[serde(default)]
    pub available_function_schemas: Vec<FunctionSchema>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorStatus {
    Selected,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectorAction {
    WorkflowStart,
    WorkflowStatus,
    DiagnosticsInvestigate,
    CommandInvoke,
    NoResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectorResult {
    pub selector_id: String,
    pub status: SelectorStatus,
    #[serde(default)]
    pub action: Option<SelectorAction>,
    #[serde(default)]
    pub selected_workflow: Option<String>,
    #[serde(default)]
    pub diagnostics_scope: Option<Map<String, Value>>,
    #[serde(default)]
    pub function_id: Option<String>,
    #[serde(default)]
    pub function_args: Option<Map<String, Value>>,
    #[serde(default)]
    pub reason: Option<String>,
}

pub fn parse_and_validate_selector_result(
    raw_json: &str,
    request: &SelectorRequest,
) -> Result<SelectorResult, OrchestratorError> {
    let result: SelectorResult = serde_json::from_str(raw_json)
        .map_err(|e| OrchestratorError::SelectorJson(e.to_string()))?;

    if result.selector_id != request.selector_id {
        return Err(OrchestratorError::SelectorValidation(
            "selectorId mismatch".to_string(),
        ));
    }

    match result.status {
        SelectorStatus::Failed => Ok(result),
        SelectorStatus::Selected => {
            let action = result.action.ok_or_else(|| {
                OrchestratorError::SelectorValidation("selected result requires action".to_string())
            })?;
            match action {
                SelectorAction::WorkflowStart => {
                    let selected = result.selected_workflow.as_ref().ok_or_else(|| {
                        OrchestratorError::SelectorValidation(
                            "workflow_start requires selectedWorkflow".to_string(),
                        )
                    })?;
                    if !request.available_workflows.iter().any(|v| v == selected) {
                        return Err(OrchestratorError::SelectorValidation(format!(
                            "workflow `{selected}` is not in availableWorkflows"
                        )));
                    }
                }
                SelectorAction::WorkflowStatus => {}
                SelectorAction::DiagnosticsInvestigate => {
                    if result.diagnostics_scope.is_none() {
                        return Err(OrchestratorError::SelectorValidation(
                            "diagnostics_investigate requires diagnosticsScope object".to_string(),
                        ));
                    }
                }
                SelectorAction::CommandInvoke => {
                    let function_id = result.function_id.as_ref().ok_or_else(|| {
                        OrchestratorError::SelectorValidation(
                            "command_invoke requires functionId".to_string(),
                        )
                    })?;
                    if !request.available_functions.iter().any(|f| f == function_id) {
                        return Err(OrchestratorError::SelectorValidation(format!(
                            "function `{function_id}` is not in availableFunctions"
                        )));
                    }
                    if result.function_args.is_none() {
                        return Err(OrchestratorError::SelectorValidation(
                            "command_invoke requires functionArgs object".to_string(),
                        ));
                    }
                    if let Some(schema) = request
                        .available_function_schemas
                        .iter()
                        .find(|schema| schema.function_id == *function_id)
                    {
                        let args = result.function_args.as_ref().expect("checked above");
                        for key in args.keys() {
                            if !schema.args.contains_key(key) {
                                return Err(OrchestratorError::SelectorValidation(format!(
                                    "command_invoke has unknown argument `{key}` for function `{function_id}`"
                                )));
                            }
                        }
                        for (arg, arg_schema) in &schema.args {
                            match args.get(arg) {
                                Some(value) if arg_schema.arg_type.matches(value) => {}
                                Some(_) => {
                                    return Err(OrchestratorError::SelectorValidation(format!(
                                        "command_invoke argument `{arg}` for function `{function_id}` must be {}",
                                        arg_schema.arg_type
                                    )))
                                }
                                None if arg_schema.required => {
                                    return Err(OrchestratorError::SelectorValidation(format!(
                                        "command_invoke missing required argument `{arg}` for function `{function_id}`"
                                    )))
                                }
                                None => {}
                            }
                        }
                    }
                }
                SelectorAction::NoResponse => {}
            }
            Ok(result)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionResolution {
    pub result: SelectorResult,
    pub retries_used: u32,
    pub fell_back_to_default_workflow: bool,
}

pub fn resolve_selector_with_retries<F>(
    orchestrator: &OrchestratorConfig,
    request: &SelectorRequest,
    mut next_attempt: F,
) -> SelectionResolution
where
    F: FnMut(u32) -> Option<String>,
{
    let max_attempts = orchestrator.selection_max_retries.saturating_add(1);
    let mut attempt = 0_u32;
    while attempt < max_attempts {
        let raw = next_attempt(attempt);
        if let Some(raw) = raw {
            if let Ok(validated) = parse_and_validate_selector_result(&raw, request) {
                if validated.status == SelectorStatus::Selected {
                    return SelectionResolution {
                        result: validated,
                        retries_used: attempt,
                        fell_back_to_default_workflow: false,
                    };
                }
            }
        }
        attempt += 1;
    }

    SelectionResolution {
        result: SelectorResult {
            selector_id: request.selector_id.clone(),
            status: SelectorStatus::Selected,
            action: Some(SelectorAction::WorkflowStart),
            selected_workflow: Some(orchestrator.default_workflow.clone()),
            diagnostics_scope: None,
            function_id: None,
            function_args: None,
            reason: Some("fallback_to_default_workflow_after_retry_limit".to_string()),
        },
        retries_used: orchestrator.selection_max_retries,
        fell_back_to_default_workflow: true,
    }
}
