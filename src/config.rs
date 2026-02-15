use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub mod paths;
pub(crate) mod setup_draft;
pub mod typed_fields;
pub use paths::{
    default_global_config_path, default_orchestrators_config_path, load_global_settings,
    GLOBAL_ORCHESTRATORS_FILE_NAME, GLOBAL_SETTINGS_FILE_NAME, GLOBAL_STATE_DIR,
};
pub(crate) use setup_draft::{OrchestrationLimitField, SetupDraft};
pub use typed_fields::{
    normalize_workflow_input_key, parse_output_contract_key, AgentId, OrchestratorId,
    OutputContractKey, OutputKey, PathTemplate, StepId, WorkflowId, WorkflowInputKey,
    WorkflowInputs,
};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid yaml in {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("settings validation failed: {0}")]
    Settings(String),
    #[error("orchestrator validation failed: {0}")]
    Orchestrator(String),
    #[error("orchestrator `{orchestrator_id}` is not configured in settings")]
    MissingOrchestrator { orchestrator_id: String },
    #[error("failed to resolve home directory for global config path")]
    HomeDirectoryUnavailable,
}

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
pub enum ChannelKind {
    Local,
    Slack,
    Discord,
    Telegram,
    Whatsapp,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Slack => "slack",
            Self::Discord => "discord",
            Self::Telegram => "telegram",
            Self::Whatsapp => "whatsapp",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "slack" => Ok(Self::Slack),
            "discord" => Ok(Self::Discord),
            "telegram" => Ok(Self::Telegram),
            "whatsapp" => Ok(Self::Whatsapp),
            _ => {
                Err("channel must be one of: local, slack, discord, telegram, whatsapp".to_string())
            }
        }
    }
}

impl std::fmt::Display for ChannelKind {
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Settings {
    pub workspaces_path: PathBuf,
    #[serde(default)]
    pub shared_workspaces: BTreeMap<String, PathBuf>,
    #[serde(default)]
    pub orchestrators: BTreeMap<String, SettingsOrchestrator>,
    #[serde(default)]
    pub channel_profiles: BTreeMap<String, ChannelProfile>,
    #[serde(default)]
    pub monitoring: Monitoring,
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelConfig>,
    #[serde(default)]
    pub auth_sync: AuthSyncConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SettingsOrchestrator {
    pub private_workspace: Option<PathBuf>,
    #[serde(default)]
    pub shared_access: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelProfile {
    pub channel: ChannelKind,
    pub orchestrator_id: String,
    pub slack_app_user_id: Option<String>,
    pub require_mention_in_channels: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Monitoring {
    pub heartbeat_interval: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ChannelConfig {
    pub enabled: bool,
    #[serde(default)]
    pub allowlisted_channels: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AuthSyncConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub sources: BTreeMap<String, AuthSyncSource>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthSyncSource {
    pub backend: String,
    pub reference: String,
    pub destination: PathBuf,
    #[serde(default = "default_true")]
    pub owner_only: bool,
}

fn default_true() -> bool {
    true
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

#[derive(Debug, Clone, Copy)]
pub struct ValidationOptions {
    pub require_shared_paths_exist: bool,
}

impl Default for ValidationOptions {
    fn default() -> Self {
        Self {
            require_shared_paths_exist: true,
        }
    }
}

impl Settings {
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

    pub fn validate(&self, options: ValidationOptions) -> Result<(), ConfigError> {
        if !self.workspaces_path.is_absolute() {
            return Err(ConfigError::Settings(
                "`workspaces_path` must be an absolute path".to_string(),
            ));
        }

        let mut shared_keys = HashSet::new();
        for (key, path) in &self.shared_workspaces {
            if key.trim().is_empty() {
                return Err(ConfigError::Settings(
                    "`shared_workspaces` keys must be non-empty".to_string(),
                ));
            }
            if !path.is_absolute() {
                return Err(ConfigError::Settings(format!(
                    "shared workspace `{key}` must use an absolute path"
                )));
            }
            if options.require_shared_paths_exist {
                fs::canonicalize(path).map_err(|_| {
                    ConfigError::Settings(format!(
                        "shared workspace `{key}` path `{}` is missing or invalid",
                        path.display()
                    ))
                })?;
            }
            shared_keys.insert(key.clone());
        }

        for (orchestrator_id, orchestrator) in &self.orchestrators {
            OrchestratorId::parse(orchestrator_id).map_err(ConfigError::Settings)?;
            for grant in &orchestrator.shared_access {
                if !shared_keys.contains(grant) {
                    return Err(ConfigError::Settings(format!(
                        "orchestrator `{orchestrator_id}` references unknown shared workspace `{grant}`"
                    )));
                }
            }
        }

        for (profile_id, profile) in &self.channel_profiles {
            if profile_id.trim().is_empty() {
                return Err(ConfigError::Settings(
                    "channel profile id must be non-empty".to_string(),
                ));
            }
            if !self.orchestrators.contains_key(&profile.orchestrator_id) {
                return Err(ConfigError::Settings(format!(
                    "channel profile `{profile_id}` references unknown orchestrator `{}`",
                    profile.orchestrator_id
                )));
            }
            if profile.channel == ChannelKind::Slack {
                if profile
                    .slack_app_user_id
                    .as_ref()
                    .map(|v| v.trim().is_empty())
                    .unwrap_or(true)
                {
                    return Err(ConfigError::Settings(format!(
                        "slack profile `{profile_id}` requires `slack_app_user_id`"
                    )));
                }
                if profile.require_mention_in_channels.is_none() {
                    return Err(ConfigError::Settings(format!(
                        "slack profile `{profile_id}` requires `require_mention_in_channels`"
                    )));
                }
            }
        }

        if let Some(slack_cfg) = self.channels.get("slack") {
            for channel_id in &slack_cfg.allowlisted_channels {
                if channel_id.trim().is_empty() {
                    return Err(ConfigError::Settings(
                        "channels.slack.allowlisted_channels entries must be non-empty".to_string(),
                    ));
                }
            }
        }

        if self.auth_sync.enabled {
            if self.auth_sync.sources.is_empty() {
                return Err(ConfigError::Settings(
                    "`auth_sync.sources` must be non-empty when `auth_sync.enabled=true`"
                        .to_string(),
                ));
            }
            for (source_id, source) in &self.auth_sync.sources {
                if source_id.trim().is_empty() {
                    return Err(ConfigError::Settings(
                        "`auth_sync.sources` keys must be non-empty".to_string(),
                    ));
                }
                if source.backend.trim().is_empty() {
                    return Err(ConfigError::Settings(format!(
                        "auth sync source `{source_id}` requires non-empty `backend`"
                    )));
                }
                if source.backend.trim() != "onepassword" {
                    return Err(ConfigError::Settings(format!(
                        "auth sync source `{source_id}` has unsupported backend `{}`",
                        source.backend
                    )));
                }
                if source.reference.trim().is_empty() {
                    return Err(ConfigError::Settings(format!(
                        "auth sync source `{source_id}` requires non-empty `reference`"
                    )));
                }
                let destination_raw = source.destination.to_string_lossy();
                let valid_destination =
                    source.destination.is_absolute() || destination_raw.starts_with("~/");
                if !valid_destination {
                    return Err(ConfigError::Settings(format!(
                        "auth sync source `{source_id}` destination `{}` must be absolute or start with `~/`",
                        source.destination.display()
                    )));
                }
            }
        }

        Ok(())
    }

    pub fn resolve_private_workspace(&self, orchestrator_id: &str) -> Result<PathBuf, ConfigError> {
        OrchestratorId::parse(orchestrator_id).map_err(ConfigError::Settings)?;
        let orchestrator = self.orchestrators.get(orchestrator_id).ok_or_else(|| {
            ConfigError::MissingOrchestrator {
                orchestrator_id: orchestrator_id.to_string(),
            }
        })?;

        let resolved = if let Some(override_path) = &orchestrator.private_workspace {
            override_path.clone()
        } else {
            self.workspaces_path.join(orchestrator_id)
        };

        if !resolved.is_absolute() {
            return Err(ConfigError::Settings(format!(
                "resolved private workspace for `{orchestrator_id}` is not absolute"
            )));
        }

        Ok(resolved)
    }
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

pub fn load_orchestrator_config(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<OrchestratorConfig, ConfigError> {
    if !settings.orchestrators.contains_key(orchestrator_id) {
        return Err(ConfigError::MissingOrchestrator {
            orchestrator_id: orchestrator_id.to_string(),
        });
    }
    let workspace = settings.resolve_private_workspace(orchestrator_id)?;
    let workspaces_path = workspace.join("orchestrator.yaml");
    let config = OrchestratorConfig::from_path(&workspaces_path)?;
    config.validate(settings, orchestrator_id)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn private_workspace_override_wins() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    private_workspace: /tmp/custom-alpha
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let resolved = settings
            .resolve_private_workspace("alpha")
            .expect("resolve workspace");
        assert_eq!(resolved, PathBuf::from("/tmp/custom-alpha"));
    }

    #[test]
    fn private_workspace_falls_back_to_default_rule() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let resolved = settings
            .resolve_private_workspace("alpha")
            .expect("resolve workspace");
        assert_eq!(resolved, PathBuf::from("/tmp/workspace/alpha"));
    }

    #[test]
    fn settings_validation_fails_for_unknown_shared_grant() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces:
  docs: /tmp/docs
orchestrators:
  alpha:
    shared_access: [missing]
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let err = settings
            .validate(ValidationOptions {
                require_shared_paths_exist: false,
            })
            .expect_err("validation should fail");
        match err {
            ConfigError::Settings(message) => {
                assert!(message.contains("unknown shared workspace"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn settings_validation_canonicalizes_and_requires_shared_paths_when_enabled() {
        let temp = tempdir().expect("temp dir");
        let docs = temp.path().join("docs");
        fs::create_dir_all(&docs).expect("create docs path");

        let yaml = format!(
            r#"
workspaces_path: {workspace}
shared_workspaces:
  docs: {docs}
orchestrators:
  alpha:
    shared_access: [docs]
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
            workspace = temp.path().display(),
            docs = docs.display()
        );

        let settings: Settings = serde_yaml::from_str(&yaml).expect("parse settings");
        settings
            .validate(ValidationOptions::default())
            .expect("validation succeeds");
    }

    #[test]
    fn orchestrator_validation_enforces_selector_default_and_workflows() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let config: OrchestratorConfig = serde_yaml::from_str(
            r#"
id: alpha
selector_agent: router
default_workflow: missing
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
workflows:
  - id: real
    version: 1
    inputs: [user_prompt]
    steps:
      - id: step_1
        type: agent_task
        agent: router
        prompt: hello
        outputs: [summary]
        output_files:
          summary: summary.txt
"#,
        )
        .expect("parse orchestrator");

        let err = config
            .validate(&settings, "alpha")
            .expect_err("validation should fail");
        match err {
            ConfigError::Orchestrator(message) => {
                assert!(message.contains("default_workflow"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn orchestrator_validation_rejects_zero_selector_timeout_seconds() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let config: OrchestratorConfig = serde_yaml::from_str(
            r#"
id: alpha
selector_agent: router
default_workflow: real
selection_max_retries: 1
selector_timeout_seconds: 0
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
workflows:
  - id: real
    version: 1
    steps:
      - id: step_1
        type: agent_task
        agent: router
        prompt: hello
        outputs: [summary]
        output_files:
          summary: summary.txt
"#,
        )
        .expect("parse orchestrator");

        let err = config
            .validate(&settings, "alpha")
            .expect_err("validation should fail");
        match err {
            ConfigError::Orchestrator(message) => {
                assert!(message.contains("selector_timeout_seconds"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn orchestrator_validation_rejects_output_keys_with_non_trailing_optional_marker() {
        let _settings: Settings = serde_yaml::from_str(
            r#"
workspaces_path: /tmp/workspace
shared_workspaces: {}
orchestrators:
  alpha:
    shared_access: []
channel_profiles: {}
monitoring: {}
channels: {}
"#,
        )
        .expect("parse settings");

        let err = serde_yaml::from_str::<OrchestratorConfig>(
            r#"
id: alpha
selector_agent: router
default_workflow: real
selection_max_retries: 1
agents:
  router:
    provider: anthropic
    model: sonnet
    can_orchestrate_workflows: true
workflows:
  - id: real
    version: 1
    steps:
      - id: step_1
        type: agent_task
        agent: router
        prompt: hello
        outputs: [plan?draft]
        output_files:
          "plan?draft": plan.md
"#,
        )
        .expect_err("invalid output key should fail at parse");
        let message = err.to_string();
        assert!(message.contains("output key"));
        assert!(message.contains("trailing `?`"));
    }

    #[test]
    fn default_global_config_path_targets_home_direclaw_config_yaml() {
        let temp = tempdir().expect("temp dir");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());

        let path = default_global_config_path().expect("resolve global config path");
        assert_eq!(path, temp.path().join(".direclaw/config.yaml"));

        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn load_global_settings_reads_direclaw_config_yaml() {
        let temp = tempdir().expect("temp dir");
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(temp.path().join(".direclaw")).expect("create config dir");

        let config_path = temp.path().join(".direclaw/config.yaml");
        fs::write(
            &config_path,
            format!(
                r#"
workspaces_path: {}
shared_workspaces: {{}}
orchestrators: {{}}
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
"#,
                workspace.display()
            ),
        )
        .expect("write global config");

        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp.path());
        let settings = load_global_settings().expect("load global settings");
        assert_eq!(settings.workspaces_path, workspace);
        if let Some(value) = old_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn workflow_step_workspace_mode_defaults_to_orchestrator_workspace() {
        let step: WorkflowStepConfig = serde_yaml::from_str(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
outputs: [summary]
output_files:
  summary: outputs/summary.txt
"#,
        )
        .expect("parse step");
        assert_eq!(
            step.workspace_mode,
            WorkflowStepWorkspaceMode::OrchestratorWorkspace
        );
    }

    #[test]
    fn workflow_step_workspace_mode_accepts_supported_values_and_rejects_unknown() {
        let run_mode: WorkflowStepConfig = serde_yaml::from_str(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
workspace_mode: run_workspace
outputs: [summary]
output_files:
  summary: outputs/summary.txt
"#,
        )
        .expect("parse run_workspace");
        assert_eq!(
            run_mode.workspace_mode,
            WorkflowStepWorkspaceMode::RunWorkspace
        );

        let err = serde_yaml::from_str::<WorkflowStepConfig>(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
workspace_mode: unknown_mode
outputs: [summary]
output_files:
  summary: outputs/summary.txt
"#,
        )
        .expect_err("unknown workspace_mode must fail");
        assert!(err.to_string().contains("workspace_mode"));
    }

    #[test]
    fn workflow_inputs_round_trip_and_normalize_keys() {
        let workflow: WorkflowConfig = serde_yaml::from_str(
            r#"
id: triage
version: 1
inputs: [ ticket ,priority,ticket]
steps:
  - id: step_1
    type: agent_task
    agent: worker
    prompt: hello
    outputs: [summary]
    output_files:
      summary: outputs/summary.txt
"#,
        )
        .expect("parse workflow");
        let keys = workflow
            .inputs
            .as_slice()
            .iter()
            .map(|key| key.as_str().to_string())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["ticket".to_string(), "priority".to_string()]);

        let encoded = serde_yaml::to_string(&workflow).expect("encode workflow");
        assert!(encoded.contains("- ticket"));
        assert!(encoded.contains("- priority"));
    }

    #[test]
    fn workflow_inputs_reject_mapping_shape() {
        let err = serde_yaml::from_str::<WorkflowConfig>(
            r#"
id: triage
version: 1
inputs:
  ticket: true
  priority: high
steps:
  - id: step_1
    type: agent_task
    agent: worker
    prompt: hello
    outputs: [summary]
    output_files:
      summary: outputs/summary.txt
"#,
        )
        .expect_err("mapping inputs should fail");
        assert!(err.to_string().contains("sequence of string keys"));
    }

    #[test]
    fn workflow_inputs_reject_invalid_key_shapes() {
        let err = serde_yaml::from_str::<WorkflowConfig>(
            r#"
id: triage
version: 1
inputs: ["bad key"]
steps:
  - id: step_1
    type: agent_task
    agent: worker
    prompt: hello
    outputs: [summary]
    output_files:
      summary: outputs/summary.txt
"#,
        )
        .expect_err("invalid workflow input key should fail");
        assert!(err.to_string().contains("workflow input key"));
    }

    #[test]
    fn workflow_step_requires_outputs_and_output_files_fields() {
        let err = serde_yaml::from_str::<WorkflowStepConfig>(
            r#"
id: step_1
type: agent_task
agent: worker
prompt: hello
"#,
        )
        .expect_err("missing output contract fields must fail");
        let message = err.to_string();
        assert!(message.contains("outputs"));
        assert!(message.contains("output_files"));
    }

    #[test]
    fn output_contract_key_parsing_tracks_required_and_optional_markers() {
        let required = parse_output_contract_key("summary").expect("required output key");
        assert_eq!(required.name, "summary");
        assert!(required.required);

        let optional = parse_output_contract_key("artifact?").expect("optional output key");
        assert_eq!(optional.name, "artifact");
        assert!(!optional.required);

        let err = parse_output_contract_key("art?ifact")
            .expect_err("non-trailing optional marker should fail");
        assert!(err.contains("trailing `?`"));
    }

    #[test]
    fn typed_enums_round_trip_with_snake_case_yaml() {
        let agent: AgentConfig = serde_yaml::from_str(
            r#"
provider: openai
model: gpt-5.3-codex
can_orchestrate_workflows: false
"#,
        )
        .expect("parse agent");
        assert_eq!(agent.provider, ConfigProviderKind::OpenAi);
        let encoded = serde_yaml::to_string(&agent).expect("encode agent");
        assert!(encoded.contains("provider: openai"));

        let profile: ChannelProfile = serde_yaml::from_str(
            r#"
channel: slack
orchestrator_id: main
slack_app_user_id: U123
require_mention_in_channels: true
"#,
        )
        .expect("parse profile");
        assert_eq!(profile.channel, ChannelKind::Slack);
        let encoded = serde_yaml::to_string(&profile).expect("encode profile");
        assert!(encoded.contains("channel: slack"));

        let step: WorkflowStepConfig = serde_yaml::from_str(
            r#"
id: review
type: agent_review
agent: reviewer
prompt: review it
outputs: [decision,summary,feedback]
output_files:
  decision: outputs/decision.txt
  summary: outputs/summary.txt
  feedback: outputs/feedback.txt
"#,
        )
        .expect("parse step");
        assert_eq!(step.step_type, WorkflowStepType::AgentReview);
        let encoded = serde_yaml::to_string(&step).expect("encode step");
        assert!(encoded.contains("type: agent_review"));
    }

    #[test]
    fn typed_enums_reject_invalid_values_with_parse_errors() {
        let provider_err = serde_yaml::from_str::<AgentConfig>(
            r#"
provider: invalid
model: sonnet
"#,
        )
        .expect_err("invalid provider should fail");
        assert!(provider_err.to_string().contains("provider"));

        let channel_err = serde_yaml::from_str::<ChannelProfile>(
            r#"
channel: invalid
orchestrator_id: main
"#,
        )
        .expect_err("invalid channel should fail");
        assert!(channel_err.to_string().contains("channel"));

        let step_err = serde_yaml::from_str::<WorkflowStepConfig>(
            r#"
id: s1
type: invalid
agent: worker
prompt: test
"#,
        )
        .expect_err("invalid step type should fail");
        assert!(step_err.to_string().contains("type"));
    }

    #[test]
    fn id_wrappers_accept_valid_and_reject_invalid_values() {
        assert!(OrchestratorId::parse("main_01").is_ok());
        assert!(WorkflowId::parse("feature-delivery").is_ok());
        assert!(StepId::parse("step_1").is_ok());
        assert!(AgentId::parse("router").is_ok());

        assert!(OrchestratorId::parse("main dev").is_err());
        assert!(WorkflowId::parse("").is_err());
        assert!(StepId::parse("step!").is_err());
        assert!(AgentId::parse("agent/id").is_err());
    }
}
