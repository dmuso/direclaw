use crate::config::{ChannelProfile, Settings, SlackInboundMode};
use crate::queue::QueuePaths;
use api::SlackApiClient;
use auth::{configured_slack_allowlist, load_env_config, slack_profiles};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

pub mod api;
pub mod auth;
pub mod cursor_store;
pub mod egress;
pub mod history_backfill;
pub mod ingest;
pub mod socket;
pub mod socket_ingest;

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
    #[error("invalid slack targetRef in outgoing message `{message_id}`: {reason}")]
    InvalidTargetRef { message_id: String, reason: String },
    #[error("unknown slack channel profile `{0}` in outgoing message")]
    UnknownChannelProfile(String),
    #[error("outgoing slack message `{message_id}` has no channel_profile_id and multiple slack profiles exist")]
    MissingChannelProfileId { message_id: String },
    #[error(
        "failed to deliver outbound slack message `{message_id}` for profile `{profile_id}` to channel `{channel_id}` thread `{thread_ts}`: {reason}"
    )]
    OutboundDelivery {
        message_id: String,
        profile_id: String,
        channel_id: String,
        thread_ts: String,
        reason: String,
    },
    #[error(
        "outgoing slack message `{message_id}` for profile `{profile_id}` targets unauthorized channel `{channel_id}`"
    )]
    UnauthorizedChannelTarget {
        message_id: String,
        profile_id: String,
        channel_id: String,
    },
    #[error("slack api request failed: {0}")]
    ApiRequest(String),
    #[error("slack api rate limited for `{path}`; retry_after_seconds={retry_after_secs}")]
    RateLimited { path: String, retry_after_secs: u64 },
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SlackProfileCredentialHealth {
    pub profile_id: String,
    pub ok: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackSocketHealth {
    pub profile_id: String,
    pub connected: bool,
    pub last_event_ts: Option<String>,
    pub last_reconnect: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct SlackProfileRuntime {
    profile: ChannelProfile,
    api: SlackApiClient,
    allowlist: BTreeSet<String>,
    include_im_conversations: bool,
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

fn slack_include_im_conversations(settings: &Settings) -> bool {
    settings
        .channels
        .get("slack")
        .map(|cfg| cfg.include_im_conversations)
        .unwrap_or(true)
}

pub fn sync_once(state_root: &Path, settings: &Settings) -> Result<SlackSyncReport, SlackError> {
    sync_backfill_once(state_root, settings)
}

pub fn sync_runtime_once(
    state_root: &Path,
    settings: &Settings,
) -> Result<SlackSyncReport, SlackError> {
    sync_socket_once(state_root, settings)
}

pub fn sync_socket_once(
    state_root: &Path,
    settings: &Settings,
) -> Result<SlackSyncReport, SlackError> {
    sync_once_internal(state_root, settings, SyncMode::SocketOnly)
}

pub fn sync_backfill_once(
    state_root: &Path,
    settings: &Settings,
) -> Result<SlackSyncReport, SlackError> {
    sync_once_internal(state_root, settings, SyncMode::BackfillOnly)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncMode {
    SocketOnly,
    BackfillOnly,
}

fn sync_once_internal(
    state_root: &Path,
    settings: &Settings,
    mode: SyncMode,
) -> Result<SlackSyncReport, SlackError> {
    validate_startup_credentials(settings)?;
    let include_im_conversations = slack_include_im_conversations(settings);
    let channel_cfg = settings.channels.get("slack").cloned().unwrap_or_default();
    let run_backfill = mode == SyncMode::BackfillOnly && channel_cfg.history_backfill_enabled;
    let run_socket = mode == SyncMode::SocketOnly;
    let runtimes =
        build_profile_runtimes(settings, include_im_conversations, run_socket, run_backfill)?;

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
        if run_backfill {
            report.inbound_enqueued += history_backfill::process_inbound_for_profile(
                state_root,
                &queue_paths,
                profile_id,
                runtime,
            )?;
        }
        if run_socket {
            report.inbound_enqueued += socket_ingest::process_socket_inbound_for_profile(
                state_root,
                &queue_paths,
                profile_id,
                runtime,
                channel_cfg.socket_reconnect_backoff_ms,
                channel_cfg.socket_idle_timeout_ms,
            )?;
        }
        outbound_roots.insert(runtime_root);
    }
    for runtime_root in outbound_roots {
        let queue_paths = QueuePaths::from_state_root(&runtime_root);
        report.outbound_messages_sent += egress::process_outbound(&queue_paths, &runtimes)?;
    }
    Ok(report)
}

fn build_profile_runtimes(
    settings: &Settings,
    include_im_conversations: bool,
    validate_socket_auth: bool,
    validate_backfill_connection: bool,
) -> Result<BTreeMap<String, SlackProfileRuntime>, SlackError> {
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
        if validate_socket_auth {
            api.validate_auth()?;
        } else if validate_backfill_connection {
            api.validate_connection()?;
        }
        runtimes.insert(
            profile_id,
            SlackProfileRuntime {
                profile,
                api,
                allowlist: env.allowlist,
                include_im_conversations,
            },
        );
    }

    Ok(runtimes)
}

pub fn run_socket_runtime_until_stop(
    state_root: &Path,
    settings: &Settings,
    stop: Arc<AtomicBool>,
) -> Result<(), SlackError> {
    validate_startup_credentials(settings)?;
    let channel_cfg = settings.channels.get("slack").cloned().unwrap_or_default();
    let reconnect_backoff_ms = channel_cfg.socket_reconnect_backoff_ms;
    let runtimes = build_profile_runtimes(
        settings,
        slack_include_im_conversations(settings),
        true,
        false,
    )?;

    let mut outbound_roots = BTreeSet::<PathBuf>::new();
    let (result_tx, result_rx) = mpsc::channel::<Result<(), SlackError>>();
    let mut handles = Vec::new();
    let runtimes_for_sockets = Arc::new(runtimes.clone());

    for (profile_id, runtime) in runtimes.clone() {
        let runtime_root = settings
            .resolve_channel_profile_runtime_root(&profile_id)
            .map_err(|err| SlackError::Config(err.to_string()))?;
        let queue_paths = QueuePaths::from_state_root(&runtime_root);
        fs::create_dir_all(&queue_paths.incoming)
            .map_err(|e| io_error(&queue_paths.incoming, e))?;
        fs::create_dir_all(&queue_paths.outgoing)
            .map_err(|e| io_error(&queue_paths.outgoing, e))?;
        outbound_roots.insert(runtime_root);

        let root = state_root.to_path_buf();
        let profile = profile_id.clone();
        let stop_for_socket = Arc::clone(&stop);
        let tx = result_tx.clone();
        handles.push(thread::spawn(move || {
            let outcome = socket_ingest::run_socket_inbound_for_profile_until_stop(
                &root,
                &queue_paths,
                &profile,
                &runtime,
                reconnect_backoff_ms,
                stop_for_socket.as_ref(),
            )
            .map(|_| ());
            let _ = tx.send(outcome);
        }));
    }
    drop(result_tx);

    let outbound_interval = Duration::from_secs(1);
    while !stop.load(Ordering::Relaxed) {
        for runtime_root in &outbound_roots {
            let queue_paths = QueuePaths::from_state_root(runtime_root);
            let _ = egress::process_outbound(&queue_paths, runtimes_for_sockets.as_ref())?;
        }

        while let Ok(outcome) = result_rx.try_recv() {
            if let Err(err) = outcome {
                stop.store(true, Ordering::Relaxed);
                for handle in handles {
                    let _ = handle.join();
                }
                return Err(err);
            }
        }

        thread::sleep(outbound_interval);
    }

    for handle in handles {
        let _ = handle.join();
    }
    Ok(())
}

pub fn inbound_mode(settings: &Settings) -> SlackInboundMode {
    settings
        .channels
        .get("slack")
        .map(|cfg| cfg.inbound_mode)
        .unwrap_or_default()
}

pub fn socket_health(state_root: &Path, settings: &Settings) -> Vec<SlackSocketHealth> {
    let mut health = Vec::new();
    for profile_id in slack_profiles(settings).keys() {
        let profile_health = socket::read_profile_health(state_root, profile_id);
        health.push(SlackSocketHealth {
            profile_id: profile_id.clone(),
            connected: profile_health.connected,
            last_event_ts: profile_health.last_event_ts,
            last_reconnect: profile_health.last_reconnect,
            last_error: profile_health.last_error,
        });
    }
    health
}

pub fn request_socket_reconnect(state_root: &Path) -> Result<(), SlackError> {
    socket::request_reconnect(state_root)
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
