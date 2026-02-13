use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

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
}

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub workspace_path: PathBuf,
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsOrchestrator {
    pub private_workspace: Option<PathBuf>,
    #[serde(default)]
    pub shared_access: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelProfile {
    pub channel: String,
    pub orchestrator_id: String,
    pub slack_app_user_id: Option<String>,
    pub require_mention_in_channels: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Monitoring {
    pub heartbeat_interval: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChannelConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrchestratorConfig {
    pub id: String,
    pub selector_agent: String,
    pub default_workflow: String,
    pub selection_max_retries: u32,
    pub agents: BTreeMap<String, AgentConfig>,
    #[serde(default)]
    pub workflows: Vec<WorkflowConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub private_workspace: Option<PathBuf>,
    #[serde(default)]
    pub can_orchestrate_workflows: bool,
    #[serde(default)]
    pub shared_access: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowConfig {
    pub id: String,
    pub version: u32,
    #[serde(default)]
    pub inputs: serde_yaml::Value,
    #[serde(default)]
    pub steps: Vec<WorkflowStepConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowStepConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub step_type: String,
    pub agent: String,
    pub prompt: String,
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
        if !self.workspace_path.is_absolute() {
            return Err(ConfigError::Settings(
                "`workspace_path` must be an absolute path".to_string(),
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
            for grant in &orchestrator.shared_access {
                if !shared_keys.contains(grant) {
                    return Err(ConfigError::Settings(format!(
                        "orchestrator `{orchestrator_id}` references unknown shared workspace `{grant}`"
                    )));
                }
            }
        }

        for (profile_id, profile) in &self.channel_profiles {
            if !self.orchestrators.contains_key(&profile.orchestrator_id) {
                return Err(ConfigError::Settings(format!(
                    "channel profile `{profile_id}` references unknown orchestrator `{}`",
                    profile.orchestrator_id
                )));
            }
            if profile.channel == "slack" {
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

        Ok(())
    }

    pub fn resolve_private_workspace(&self, orchestrator_id: &str) -> Result<PathBuf, ConfigError> {
        let orchestrator = self.orchestrators.get(orchestrator_id).ok_or_else(|| {
            ConfigError::MissingOrchestrator {
                orchestrator_id: orchestrator_id.to_string(),
            }
        })?;

        let resolved = if let Some(override_path) = &orchestrator.private_workspace {
            override_path.clone()
        } else {
            self.workspace_path
                .join("orchestrators")
                .join(orchestrator_id)
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
        if self.workflows.is_empty() {
            return Err(ConfigError::Orchestrator(
                "`workflows` must be non-empty".to_string(),
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

        let workflow_ids: HashSet<&str> = self.workflows.iter().map(|w| w.id.as_str()).collect();
        if !workflow_ids.contains(self.default_workflow.as_str()) {
            return Err(ConfigError::Orchestrator(format!(
                "`default_workflow` `{}` is not present in `workflows`",
                self.default_workflow
            )));
        }

        let orchestrator_grants = settings
            .orchestrators
            .get(orchestrator_id)
            .map(|entry| entry.shared_access.iter().cloned().collect::<HashSet<_>>())
            .ok_or_else(|| ConfigError::MissingOrchestrator {
                orchestrator_id: orchestrator_id.to_string(),
            })?;

        for (agent_id, agent) in &self.agents {
            if agent.provider.trim().is_empty() {
                return Err(ConfigError::Orchestrator(format!(
                    "agent `{agent_id}` requires non-empty `provider`"
                )));
            }
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
            if workflow.steps.is_empty() {
                return Err(ConfigError::Orchestrator(format!(
                    "workflow `{}` requires at least one step",
                    workflow.id
                )));
            }
            for step in &workflow.steps {
                if !self.agents.contains_key(&step.agent) {
                    return Err(ConfigError::Orchestrator(format!(
                        "workflow `{}` step `{}` references unknown agent `{}`",
                        workflow.id, step.id, step.agent
                    )));
                }
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
}

pub fn load_orchestrator_config(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<OrchestratorConfig, ConfigError> {
    let workspace = settings.resolve_private_workspace(orchestrator_id)?;
    let path = workspace.join("orchestrator.yaml");
    let config = OrchestratorConfig::from_path(&path)?;
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
workspace_path: /tmp/workspace
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
workspace_path: /tmp/workspace
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
        assert_eq!(
            resolved,
            PathBuf::from("/tmp/workspace/orchestrators/alpha")
        );
    }

    #[test]
    fn settings_validation_fails_for_unknown_shared_grant() {
        let settings: Settings = serde_yaml::from_str(
            r#"
workspace_path: /tmp/workspace
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
workspace_path: {workspace}
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
workspace_path: /tmp/workspace
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
}
