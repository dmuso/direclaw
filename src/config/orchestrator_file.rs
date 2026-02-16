use super::{
    AgentId, ConfigError, OrchestratorId, OutputKey, PathTemplate, Settings, StepId, WorkflowId,
    WorkflowInputs,
};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

fn deserialize_optional_output_files<'de, D>(
    deserializer: D,
) -> Result<Option<BTreeMap<OutputKey, PathTemplate>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<BTreeMap<String, PathTemplate>>::deserialize(deserializer)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut parsed = BTreeMap::new();
    for (key, template) in raw {
        let parsed_key = OutputKey::parse_output_file_key(&key)
            .map_err(|err| D::Error::custom(format!("invalid output_files key `{key}`: {err}")))?;
        parsed.insert(parsed_key, template);
    }
    Ok(Some(parsed))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigProviderKind {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
}

impl ConfigProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            _ => Err("provider must be one of: anthropic, openai".to_string()),
        }
    }
}

impl std::fmt::Display for ConfigProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepType {
    AgentTask,
    AgentReview,
}

impl WorkflowStepType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentTask => "agent_task",
            Self::AgentReview => "agent_review",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "agent_task" => Ok(Self::AgentTask),
            "agent_review" => Ok(Self::AgentReview),
            _ => Err("step type must be one of: agent_task, agent_review".to_string()),
        }
    }
}

impl std::fmt::Display for WorkflowStepType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

fn default_selector_timeout_seconds() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrchestratorConfig {
    pub id: String,
    pub selector_agent: String,
    pub default_workflow: String,
    pub selection_max_retries: u32,
    #[serde(default = "default_selector_timeout_seconds")]
    pub selector_timeout_seconds: u64,
    pub agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    pub workflows: Vec<WorkflowConfig>,
    #[serde(default)]
    pub workflow_orchestration: Option<WorkflowOrchestrationConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    pub provider: ConfigProviderKind,
    pub model: String,
    #[serde(default)]
    pub private_workspace: Option<PathBuf>,
    #[serde(default)]
    pub can_orchestrate_workflows: bool,
    #[serde(default)]
    pub shared_access: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEditableField {
    Provider,
    Model,
    PrivateWorkspace,
    SharedAccess,
    CanOrchestrateWorkflows,
}

const AGENT_EDITABLE_FIELDS: [AgentEditableField; 5] = [
    AgentEditableField::Provider,
    AgentEditableField::Model,
    AgentEditableField::PrivateWorkspace,
    AgentEditableField::SharedAccess,
    AgentEditableField::CanOrchestrateWorkflows,
];

pub fn agent_editable_fields() -> &'static [AgentEditableField] {
    &AGENT_EDITABLE_FIELDS
}

impl AgentEditableField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Provider => "Provider",
            Self::Model => "Model",
            Self::PrivateWorkspace => "Private Workspace",
            Self::SharedAccess => "Shared Access",
            Self::CanOrchestrateWorkflows => "Can Orchestrate Workflows",
        }
    }
}

impl AgentConfig {
    pub fn display_value_for_field(&self, field: AgentEditableField) -> String {
        match field {
            AgentEditableField::Provider => self.provider.to_string(),
            AgentEditableField::Model => self.model.clone(),
            AgentEditableField::PrivateWorkspace => self
                .private_workspace
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            AgentEditableField::SharedAccess => {
                if self.shared_access.is_empty() {
                    "<none>".to_string()
                } else {
                    self.shared_access.join(",")
                }
            }
            AgentEditableField::CanOrchestrateWorkflows => {
                if self.can_orchestrate_workflows {
                    "yes".to_string()
                } else {
                    "no".to_string()
                }
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    pub id: String,
    pub version: u32,
    #[serde(default)]
    pub inputs: WorkflowInputs,
    #[serde(default)]
    pub limits: Option<WorkflowLimitsConfig>,
    #[serde(default)]
    pub steps: Vec<WorkflowStepConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowStepConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub step_type: WorkflowStepType,
    pub agent: String,
    pub prompt: String,
    #[serde(default = "default_workflow_step_prompt_type")]
    pub prompt_type: WorkflowStepPromptType,
    #[serde(default = "default_workflow_step_workspace_mode")]
    pub workspace_mode: WorkflowStepWorkspaceMode,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub on_approve: Option<String>,
    #[serde(default)]
    pub on_reject: Option<String>,
    pub outputs: Vec<OutputKey>,
    pub output_files: BTreeMap<OutputKey, PathTemplate>,
    #[serde(default)]
    pub limits: Option<StepLimitsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkflowStepConfigRaw {
    pub id: String,
    #[serde(rename = "type")]
    pub step_type: WorkflowStepType,
    pub agent: String,
    pub prompt: String,
    #[serde(default = "default_workflow_step_prompt_type")]
    pub prompt_type: WorkflowStepPromptType,
    #[serde(default = "default_workflow_step_workspace_mode")]
    pub workspace_mode: WorkflowStepWorkspaceMode,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub on_approve: Option<String>,
    #[serde(default)]
    pub on_reject: Option<String>,
    pub outputs: Option<Vec<OutputKey>>,
    #[serde(default, deserialize_with = "deserialize_optional_output_files")]
    pub output_files: Option<BTreeMap<OutputKey, PathTemplate>>,
    #[serde(default)]
    pub limits: Option<StepLimitsConfig>,
}

impl<'de> Deserialize<'de> for WorkflowStepConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = WorkflowStepConfigRaw::deserialize(deserializer)?;
        let outputs = raw.outputs.ok_or_else(|| {
            D::Error::custom(
                "workflow step is missing required `outputs`; include explicit `outputs` and `output_files` contract fields",
            )
        })?;
        let output_files = raw.output_files.ok_or_else(|| {
            D::Error::custom(
                "workflow step is missing required `output_files`; include explicit `outputs` and `output_files` contract fields",
            )
        })?;
        Ok(Self {
            id: raw.id,
            step_type: raw.step_type,
            agent: raw.agent,
            prompt: raw.prompt,
            prompt_type: raw.prompt_type,
            workspace_mode: raw.workspace_mode,
            next: raw.next,
            on_approve: raw.on_approve,
            on_reject: raw.on_reject,
            outputs,
            output_files,
            limits: raw.limits,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepPromptType {
    WorkflowResultEnvelope,
    FileOutput,
}

fn default_workflow_step_prompt_type() -> WorkflowStepPromptType {
    WorkflowStepPromptType::WorkflowResultEnvelope
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepWorkspaceMode {
    OrchestratorWorkspace,
    RunWorkspace,
    AgentWorkspace,
}

fn default_workflow_step_workspace_mode() -> WorkflowStepWorkspaceMode {
    WorkflowStepWorkspaceMode::OrchestratorWorkspace
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowOrchestrationConfig {
    #[serde(default)]
    pub max_total_iterations: Option<u32>,
    #[serde(default)]
    pub default_run_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub default_step_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub max_step_timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowLimitsConfig {
    #[serde(default)]
    pub max_total_iterations: Option<u32>,
    #[serde(default)]
    pub run_timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StepLimitsConfig {
    #[serde(default)]
    pub max_retries: Option<u32>,
}

impl OrchestratorConfig {
    pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        serde_yaml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    pub fn validate(&self, settings: &Settings, orchestrator_id: &str) -> Result<(), ConfigError> {
        OrchestratorId::parse(orchestrator_id).map_err(ConfigError::Orchestrator)?;
        if self.id != orchestrator_id {
            return Err(ConfigError::Orchestrator(format!(
                "orchestrator id mismatch: expected `{orchestrator_id}`, got `{}`",
                self.id
            )));
        }
        if self.selection_max_retries == 0 {
            return Err(ConfigError::Orchestrator(
                "`selection_max_retries` must be >= 1".to_string(),
            ));
        }
        if self.selector_timeout_seconds == 0 {
            return Err(ConfigError::Orchestrator(
                "`selector_timeout_seconds` must be >= 1".to_string(),
            ));
        }
        self.validate_setup_invariants()?;

        let orchestrator_grants = settings
            .orchestrators
            .get(orchestrator_id)
            .map(|entry| entry.shared_access.iter().cloned().collect::<HashSet<_>>())
            .ok_or_else(|| ConfigError::MissingOrchestrator {
                orchestrator_id: orchestrator_id.to_string(),
            })?;

        for (agent_id, agent) in &self.agents {
            AgentId::parse(agent_id).map_err(ConfigError::Orchestrator)?;
            if agent.model.trim().is_empty() {
                return Err(ConfigError::Orchestrator(format!(
                    "agent `{agent_id}` requires non-empty `model`"
                )));
            }
            for shared in &agent.shared_access {
                if !orchestrator_grants.contains(shared) {
                    return Err(ConfigError::Orchestrator(format!(
                        "agent `{agent_id}` shared access `{shared}` is not granted to orchestrator `{orchestrator_id}`"
                    )));
                }
            }
        }

        for workflow in &self.workflows {
            WorkflowId::parse(&workflow.id).map_err(ConfigError::Orchestrator)?;
            for step in &workflow.steps {
                StepId::parse(&step.id).map_err(ConfigError::Orchestrator)?;
                AgentId::parse(&step.agent).map_err(ConfigError::Orchestrator)?;
                if step.prompt.trim().is_empty() {
                    return Err(ConfigError::Orchestrator(format!(
                        "workflow `{}` step `{}` requires non-empty prompt",
                        workflow.id, step.id
                    )));
                }
            }
        }

        Ok(())
    }

    pub fn validate_setup_invariants(&self) -> Result<(), ConfigError> {
        if self.workflows.is_empty() {
            return Err(ConfigError::Orchestrator(
                "`workflows` must be non-empty".to_string(),
            ));
        }
        if self.agents.is_empty() {
            return Err(ConfigError::Orchestrator(
                "`agents` must be non-empty".to_string(),
            ));
        }
        if !self.agents.contains_key(&self.selector_agent) {
            return Err(ConfigError::Orchestrator(format!(
                "`selector_agent` `{}` must exist in `agents`",
                self.selector_agent
            )));
        }
        let selector = self
            .agents
            .get(&self.selector_agent)
            .expect("checked above");
        if !selector.can_orchestrate_workflows {
            return Err(ConfigError::Orchestrator(format!(
                "selector agent `{}` must set `can_orchestrate_workflows: true`",
                self.selector_agent
            )));
        }

        let mut workflow_ids = HashSet::new();
        for workflow in &self.workflows {
            if !workflow_ids.insert(workflow.id.as_str()) {
                return Err(ConfigError::Orchestrator(format!(
                    "workflow id `{}` must be unique",
                    workflow.id
                )));
            }
        }
        if !workflow_ids.contains(self.default_workflow.as_str()) {
            return Err(ConfigError::Orchestrator(format!(
                "`default_workflow` `{}` is not present in `workflows`",
                self.default_workflow
            )));
        }

        for workflow in &self.workflows {
            if workflow.steps.is_empty() {
                return Err(ConfigError::Orchestrator(format!(
                    "workflow `{}` requires at least one step",
                    workflow.id
                )));
            }
            let mut step_ids = HashSet::new();
            for step in &workflow.steps {
                if !step_ids.insert(step.id.as_str()) {
                    return Err(ConfigError::Orchestrator(format!(
                        "workflow `{}` contains duplicate step id `{}`",
                        workflow.id, step.id
                    )));
                }
                if !self.agents.contains_key(&step.agent) {
                    return Err(ConfigError::Orchestrator(format!(
                        "workflow `{}` step `{}` references unknown agent `{}`",
                        workflow.id, step.id, step.agent
                    )));
                }
                if step.outputs.is_empty() {
                    return Err(ConfigError::Orchestrator(format!(
                        "workflow `{}` step `{}` requires non-empty `outputs`",
                        workflow.id, step.id
                    )));
                }
                if step.output_files.is_empty() {
                    return Err(ConfigError::Orchestrator(format!(
                        "workflow `{}` step `{}` requires non-empty `output_files`",
                        workflow.id, step.id
                    )));
                }
                let has_summary_output = step.outputs.iter().any(|key| key.as_str() == "summary");
                if !has_summary_output {
                    return Err(ConfigError::Orchestrator(format!(
                        "workflow `{}` step `{}` requires `summary` in `outputs`",
                        workflow.id, step.id
                    )));
                }
                if !step.output_files.contains_key("summary") {
                    return Err(ConfigError::Orchestrator(format!(
                        "workflow `{}` step `{}` requires `summary` mapping in `output_files`",
                        workflow.id, step.id
                    )));
                }
                for key in &step.outputs {
                    if !step.output_files.contains_key(key.as_str()) {
                        return Err(ConfigError::Orchestrator(format!(
                            "workflow `{}` step `{}` missing output_files mapping for `{}`",
                            workflow.id, step.id, key.name
                        )));
                    }
                }
            }
        }

        Ok(())
    }
}
