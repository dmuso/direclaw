use super::{socket, SlackError, SlackProfileRuntime};
use crate::queue::QueuePaths;
use std::path::Path;

pub(super) fn process_socket_inbound_for_profile(
    state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
    reconnect_backoff_ms: u64,
    idle_timeout_ms: u64,
) -> Result<usize, SlackError> {
    socket::process_socket_inbound_for_profile(
        state_root,
        queue_paths,
        profile_id,
        runtime,
        reconnect_backoff_ms,
        idle_timeout_ms,
    )
}

pub(super) fn run_socket_inbound_for_profile_until_stop(
    state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
    reconnect_backoff_ms: u64,
    idle_timeout_ms: u64,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<usize, SlackError> {
    socket::run_socket_inbound_for_profile_until_stop(
        state_root,
        queue_paths,
        profile_id,
        runtime,
        reconnect_backoff_ms,
        idle_timeout_ms,
        stop,
    )
}
