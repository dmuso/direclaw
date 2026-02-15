use super::{SlackError, SlackProfileCredentialHealth};
use crate::config::{ChannelKind, ChannelProfile, Settings};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub(crate) struct EnvConfig {
    pub(crate) bot_token: String,
    pub(crate) app_token: String,
    pub(crate) allowlist: BTreeSet<String>,
}

fn profile_env_key(prefix: &str, profile_id: &str) -> String {
    let mapped: String = profile_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{prefix}_{mapped}")
}

fn env_var_fallback(profile_key: &str, global_key: &str) -> Option<String> {
    std::env::var(profile_key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var(global_key)
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn parse_allowlist(value: Option<String>) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    if let Some(value) = value {
        for part in value.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                result.insert(trimmed.to_string());
            }
        }
    }
    result
}

pub(crate) fn load_env_config(
    profile_id: &str,
    require_profile_scoped_tokens: bool,
    config_allowlist: &BTreeSet<String>,
) -> Result<EnvConfig, SlackError> {
    let bot_profile = profile_env_key("SLACK_BOT_TOKEN", profile_id);
    let app_profile = profile_env_key("SLACK_APP_TOKEN", profile_id);
    let allowlist_profile = profile_env_key("SLACK_CHANNEL_ALLOWLIST", profile_id);

    let bot_token = if require_profile_scoped_tokens {
        non_empty_env(&bot_profile).ok_or_else(|| SlackError::MissingProfileScopedEnvVar {
            profile_id: profile_id.to_string(),
            key: bot_profile.clone(),
        })?
    } else {
        env_var_fallback(&bot_profile, "SLACK_BOT_TOKEN").ok_or_else(|| {
            SlackError::MissingProfileScopedEnvVar {
                profile_id: profile_id.to_string(),
                key: bot_profile.clone(),
            }
        })?
    };
    let app_token = if require_profile_scoped_tokens {
        non_empty_env(&app_profile).ok_or_else(|| SlackError::MissingProfileScopedEnvVar {
            profile_id: profile_id.to_string(),
            key: app_profile.clone(),
        })?
    } else {
        env_var_fallback(&app_profile, "SLACK_APP_TOKEN").ok_or_else(|| {
            SlackError::MissingProfileScopedEnvVar {
                profile_id: profile_id.to_string(),
                key: app_profile.clone(),
            }
        })?
    };

    let mut allowlist = config_allowlist.clone();
    let env_allowlist = parse_allowlist(env_var_fallback(
        &allowlist_profile,
        "SLACK_CHANNEL_ALLOWLIST",
    ));
    allowlist.extend(env_allowlist);

    Ok(EnvConfig {
        bot_token,
        app_token,
        allowlist,
    })
}

pub(crate) fn slack_profiles(settings: &Settings) -> BTreeMap<String, ChannelProfile> {
    settings
        .channel_profiles
        .iter()
        .filter_map(|(id, profile)| {
            if profile.channel == ChannelKind::Slack {
                Some((id.clone(), profile.clone()))
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn configured_slack_allowlist(settings: &Settings) -> BTreeSet<String> {
    settings
        .channels
        .get("slack")
        .map(|cfg| {
            cfg.allowlisted_channels
                .iter()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn validate_startup_credentials(settings: &Settings) -> Result<(), SlackError> {
    if !super::slack_channel_enabled(settings) {
        return Err(SlackError::ChannelDisabled);
    }

    let profiles = slack_profiles(settings);
    if profiles.is_empty() {
        return Err(SlackError::NoSlackProfiles);
    }
    let profile_scoped_tokens_required = profiles.len() > 1;
    let config_allowlist = configured_slack_allowlist(settings);

    let mut bot_token_profile = BTreeMap::<String, String>::new();
    let mut app_token_profile = BTreeMap::<String, String>::new();
    for profile_id in profiles.keys() {
        let env = load_env_config(
            profile_id,
            profile_scoped_tokens_required,
            &config_allowlist,
        )?;
        if let Some(existing) = bot_token_profile.insert(env.bot_token.clone(), profile_id.clone())
        {
            return Err(SlackError::DuplicateProfileCredential {
                credential: "bot".to_string(),
                profile_a: existing,
                profile_b: profile_id.clone(),
            });
        }
        if let Some(existing) = app_token_profile.insert(env.app_token.clone(), profile_id.clone())
        {
            return Err(SlackError::DuplicateProfileCredential {
                credential: "app".to_string(),
                profile_a: existing,
                profile_b: profile_id.clone(),
            });
        }
    }

    Ok(())
}

pub fn profile_credential_health(settings: &Settings) -> Vec<SlackProfileCredentialHealth> {
    let profiles = slack_profiles(settings);
    let profile_scoped_tokens_required = profiles.len() > 1;
    let config_allowlist = configured_slack_allowlist(settings);
    let mut health = BTreeMap::<String, SlackProfileCredentialHealth>::new();
    let mut bot_token_profile = BTreeMap::<String, String>::new();
    let mut app_token_profile = BTreeMap::<String, String>::new();

    for profile_id in profiles.keys() {
        match load_env_config(
            profile_id,
            profile_scoped_tokens_required,
            &config_allowlist,
        ) {
            Ok(env) => {
                health.insert(
                    profile_id.clone(),
                    SlackProfileCredentialHealth {
                        profile_id: profile_id.clone(),
                        ok: true,
                        reason: None,
                    },
                );
                if let Some(existing) = bot_token_profile.insert(env.bot_token, profile_id.clone())
                {
                    let reason = SlackError::DuplicateProfileCredential {
                        credential: "bot".to_string(),
                        profile_a: existing.clone(),
                        profile_b: profile_id.clone(),
                    }
                    .to_string();
                    if let Some(entry) = health.get_mut(&existing) {
                        entry.ok = false;
                        entry.reason = Some(reason.clone());
                    }
                    if let Some(entry) = health.get_mut(profile_id) {
                        entry.ok = false;
                        entry.reason = Some(reason);
                    }
                }
                if let Some(existing) = app_token_profile.insert(env.app_token, profile_id.clone())
                {
                    let reason = SlackError::DuplicateProfileCredential {
                        credential: "app".to_string(),
                        profile_a: existing.clone(),
                        profile_b: profile_id.clone(),
                    }
                    .to_string();
                    if let Some(entry) = health.get_mut(&existing) {
                        entry.ok = false;
                        entry.reason = Some(reason.clone());
                    }
                    if let Some(entry) = health.get_mut(profile_id) {
                        entry.ok = false;
                        entry.reason = Some(reason);
                    }
                }
            }
            Err(err) => {
                health.insert(
                    profile_id.clone(),
                    SlackProfileCredentialHealth {
                        profile_id: profile_id.clone(),
                        ok: false,
                        reason: Some(err.to_string()),
                    },
                );
            }
        }
    }

    health.into_values().collect()
}
