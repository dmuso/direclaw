use super::{ingest, SlackError, SlackProfileRuntime};
use crate::queue::QueuePaths;
use std::path::Path;

pub(super) fn process_inbound_for_profile(
    state_root: &Path,
    queue_paths: &QueuePaths,
    profile_id: &str,
    runtime: &SlackProfileRuntime,
) -> Result<usize, SlackError> {
    ingest::process_inbound_for_profile(state_root, queue_paths, profile_id, runtime)
}
