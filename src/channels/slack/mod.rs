use crate::config::{ChannelProfile, Settings};
use crate::queue::QueuePaths;
use api::SlackApiClient;
use auth::{configured_slack_allowlist, load_env_config, slack_profiles};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod api;
pub mod auth;
pub mod cursor_store;
pub mod egress;
pub mod ingest;

pub use auth::{profile_credential_health, validate_startup_credentials};

#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    #[error("slack channel is disabled in settings")]
    ChannelDisabled,
    #[error("no slack channel profiles are configured")]
    NoSlackProfiles,
    #[error("missing required env var `{0}`")]
    MissingEnvVar(String),
    #[error("missing required env var `{key}` for slack profile `{profile_id}`")]
    MissingProfileScopedEnvVar { profile_id: String, key: String },
    #[error(
        "slack profiles `{profile_a}` and `{profile_b}` resolve to the same `{credential}` token; configure distinct profile-scoped credentials"
    )]
    DuplicateProfileCredential {
        credential: String,
        profile_a: String,
        profile_b: String,
    },
    #[error("invalid conversation id `{0}` for slack outgoing message")]
    InvalidConversationId(String),
    #[error("unknown slack channel profile `{0}` in outgoing message")]
    UnknownChannelProfile(String),
    #[error("outgoing slack message `{message_id}` has no channel_profile_id and multiple slack profiles exist")]
    MissingChannelProfileId { message_id: String },
    #[error(
        "failed to deliver outbound slack message `{message_id}` for profile `{profile_id}`: {reason}"
    )]
    OutboundDelivery {
        message_id: String,
        profile_id: String,
        reason: String,
    },
    #[error("slack api request failed: {0}")]
    ApiRequest(String),
    #[error("slack api responded with error `{0}`")]
    ApiResponse(String),
    #[error("invalid settings configuration: {0}")]
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlackSyncReport {
    pub profiles_processed: usize,
    pub inbound_enqueued: usize,
    pub outbound_messages_sent: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackProfileCredentialHealth {
    pub profile_id: String,
    pub ok: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
struct SlackProfileRuntime {
    profile: ChannelProfile,
    api: SlackApiClient,
    allowlist: BTreeSet<String>,
}

fn io_error(path: &Path, source: std::io::Error) -> SlackError {
    SlackError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn json_error(path: &Path, source: serde_json::Error) -> SlackError {
    SlackError::Json {
        path: path.display().to_string(),
        source,
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn sanitize_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn slack_channel_enabled(settings: &Settings) -> bool {
    settings
        .channels
        .get("slack")
        .map(|cfg| cfg.enabled)
        .unwrap_or(false)
}

pub fn sync_once(state_root: &Path, settings: &Settings) -> Result<SlackSyncReport, SlackError> {
    validate_startup_credentials(settings)?;
    let profiles = slack_profiles(settings);
    let profile_scoped_tokens_required = profiles.len() > 1;
    let config_allowlist = configured_slack_allowlist(settings);

    let mut bot_token_profile = BTreeMap::<String, String>::new();
    let mut app_token_profile = BTreeMap::<String, String>::new();
    let mut runtimes = BTreeMap::new();
    for (profile_id, profile) in profiles {
        let env = load_env_config(
            &profile_id,
            profile_scoped_tokens_required,
            &config_allowlist,
        )?;
        if let Some(existing) = bot_token_profile.insert(env.bot_token.clone(), profile_id.clone())
        {
            return Err(SlackError::DuplicateProfileCredential {
                credential: "bot".to_string(),
                profile_a: existing,
                profile_b: profile_id,
            });
        }
        if let Some(existing) = app_token_profile.insert(env.app_token.clone(), profile_id.clone())
        {
            return Err(SlackError::DuplicateProfileCredential {
                credential: "app".to_string(),
                profile_a: existing,
                profile_b: profile_id,
            });
        }
        let api = SlackApiClient::new(env.bot_token, env.app_token);
        api.validate_connection()?;
        runtimes.insert(
            profile_id,
            SlackProfileRuntime {
                profile,
                api,
                allowlist: env.allowlist,
            },
        );
    }

    let mut report = SlackSyncReport {
        profiles_processed: runtimes.len(),
        ..SlackSyncReport::default()
    };

    let mut outbound_roots = BTreeSet::<PathBuf>::new();
    for (profile_id, runtime) in &runtimes {
        let runtime_root = settings
            .resolve_channel_profile_runtime_root(profile_id)
            .map_err(|err| SlackError::Config(err.to_string()))?;
        let queue_paths = QueuePaths::from_state_root(&runtime_root);
        fs::create_dir_all(&queue_paths.incoming)
            .map_err(|e| io_error(&queue_paths.incoming, e))?;
        fs::create_dir_all(&queue_paths.outgoing)
            .map_err(|e| io_error(&queue_paths.outgoing, e))?;
        report.inbound_enqueued +=
            ingest::process_inbound_for_profile(state_root, &queue_paths, profile_id, runtime)?;
        outbound_roots.insert(runtime_root);
    }
    for runtime_root in outbound_roots {
        let queue_paths = QueuePaths::from_state_root(&runtime_root);
        report.outbound_messages_sent += egress::process_outbound(&queue_paths, &runtimes)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cursor_state_round_trip_is_stable() {
        let temp = tempdir().expect("tempdir");
        let state_root = temp.path().join(".direclaw");
        let mut state = cursor_store::SlackCursorState::default();
        state
            .conversations
            .insert("C123".to_string(), "1700000000.1".to_string());
        cursor_store::save_cursor_state(&state_root, "profile.main", &state).expect("save");
        let loaded = cursor_store::load_cursor_state(&state_root, "profile.main").expect("load");
        assert_eq!(loaded, state);
    }
}
