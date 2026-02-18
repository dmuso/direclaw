use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IncomingMessage {
    pub channel: String,
    #[serde(default)]
    pub channel_profile_id: Option<String>,
    pub sender: String,
    pub sender_id: String,
    pub message: String,
    pub timestamp: i64,
    pub message_id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub workflow_run_id: Option<String>,
    #[serde(default)]
    pub workflow_step_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingMessage {
    pub channel: String,
    #[serde(default)]
    pub channel_profile_id: Option<String>,
    pub sender: String,
    pub message: String,
    pub original_message: String,
    pub timestamp: i64,
    pub message_id: String,
    pub agent: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub target_ref: Option<Value>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub workflow_run_id: Option<String>,
    #[serde(default)]
    pub workflow_step_id: Option<String>,
}
