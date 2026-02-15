use crate::orchestration::run_store::RunState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSnapshot {
    pub run_id: String,
    pub workflow_id: String,
    pub state: RunState,
    #[serde(default)]
    pub input_count: usize,
    #[serde(default)]
    pub input_keys: Vec<String>,
    #[serde(default)]
    pub current_step_id: Option<String>,
    #[serde(default)]
    pub current_attempt: Option<u32>,
    pub started_at: i64,
    pub updated_at: i64,
    pub last_progress_at: i64,
    pub summary: String,
    pub pending_human_input: bool,
    pub next_expected_action: String,
}
