use super::{ConfigError, OrchestratorId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

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
