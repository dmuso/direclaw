use super::SlackError;
use serde::{Deserialize, Serialize};
use serde_json::json;

const DEFAULT_SLACK_API_BASE: &str = "https://slack.com/api";

#[derive(Debug, Clone)]
pub struct SlackApiClient {
    api_base: String,
    bot_token: String,
    app_token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEnvelope<T> {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(flatten)]
    data: T,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct EmptyData {}

#[derive(Debug, Clone, Deserialize)]
struct OpenConnectionData {
    #[allow(dead_code)]
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ConversationsListData {
    conversations: Vec<ConversationSummary>,
    #[serde(default)]
    response_metadata: ResponseMetadata,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ResponseMetadata {
    #[serde(default)]
    next_cursor: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ConversationSummary {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) is_im: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ConversationsHistoryData {
    #[serde(default)]
    messages: Vec<SlackMessage>,
    #[serde(default)]
    response_metadata: ResponseMetadata,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SlackMessage {
    #[serde(default)]
    pub(crate) ts: String,
    #[serde(default)]
    pub(crate) thread_ts: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
    #[serde(default)]
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) subtype: Option<String>,
    #[serde(default)]
    pub(crate) bot_id: Option<String>,
}

impl SlackApiClient {
    pub(crate) fn new(bot_token: String, app_token: String) -> Self {
        let api_base = std::env::var("DIRECLAW_SLACK_API_BASE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SLACK_API_BASE.to_string());
        Self {
            api_base,
            bot_token,
            app_token,
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.api_base.trim_end_matches('/'), path)
    }

    fn get_with_token<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        query: &[(&str, String)],
        token: &str,
    ) -> Result<T, SlackError> {
        let mut url = self.endpoint(path);
        if !query.is_empty() {
            let encoded = query
                .iter()
                .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            url = format!("{url}?{encoded}");
        }

        let response = ureq::get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|e| SlackError::ApiRequest(e.to_string()))?;

        response
            .into_json::<T>()
            .map_err(|e| SlackError::ApiRequest(e.to_string()))
    }

    fn post_json_with_token<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
        token: &str,
    ) -> Result<T, SlackError> {
        let url = self.endpoint(path);
        let response = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(
                serde_json::to_value(body).map_err(|e| SlackError::ApiRequest(e.to_string()))?,
            )
            .map_err(|e| SlackError::ApiRequest(e.to_string()))?;

        response
            .into_json::<T>()
            .map_err(|e| SlackError::ApiRequest(e.to_string()))
    }

    pub(crate) fn validate_connection(&self) -> Result<(), SlackError> {
        let auth: SlackEnvelope<EmptyData> =
            self.get_with_token("auth.test", &[], &self.bot_token)?;
        if !auth.ok {
            return Err(SlackError::ApiResponse(
                auth.error.unwrap_or_else(|| "auth.test failed".to_string()),
            ));
        }

        let conn: SlackEnvelope<OpenConnectionData> =
            self.post_json_with_token("apps.connections.open", &json!({}), &self.app_token)?;
        if !conn.ok {
            return Err(SlackError::ApiResponse(
                conn.error
                    .unwrap_or_else(|| "apps.connections.open failed".to_string()),
            ));
        }

        Ok(())
    }

    pub(crate) fn list_conversations(&self) -> Result<Vec<ConversationSummary>, SlackError> {
        let mut all = Vec::new();
        let mut cursor = String::new();
        loop {
            let mut query = vec![
                ("types", "im,public_channel,private_channel".to_string()),
                ("limit", "200".to_string()),
            ];
            if !cursor.is_empty() {
                query.push(("cursor", cursor.clone()));
            }

            let envelope: SlackEnvelope<ConversationsListData> =
                self.get_with_token("conversations.list", &query, &self.bot_token)?;
            if !envelope.ok {
                return Err(SlackError::ApiResponse(
                    envelope
                        .error
                        .unwrap_or_else(|| "conversations.list failed".to_string()),
                ));
            }
            let data = envelope.data;
            all.extend(data.conversations);
            cursor = data.response_metadata.next_cursor;
            if cursor.trim().is_empty() {
                break;
            }
        }
        Ok(all)
    }

    pub(crate) fn conversation_history(
        &self,
        conversation_id: &str,
        oldest: Option<&str>,
    ) -> Result<Vec<SlackMessage>, SlackError> {
        let mut all = Vec::new();
        let mut cursor = String::new();
        loop {
            let mut query = vec![
                ("channel", conversation_id.to_string()),
                ("inclusive", "false".to_string()),
                ("limit", "200".to_string()),
            ];
            if let Some(oldest) = oldest {
                if !oldest.trim().is_empty() {
                    query.push(("oldest", oldest.to_string()));
                }
            }
            if !cursor.is_empty() {
                query.push(("cursor", cursor.clone()));
            }
            let envelope: SlackEnvelope<ConversationsHistoryData> =
                self.get_with_token("conversations.history", &query, &self.bot_token)?;
            if !envelope.ok {
                return Err(SlackError::ApiResponse(
                    envelope
                        .error
                        .unwrap_or_else(|| "conversations.history failed".to_string()),
                ));
            }
            let data = envelope.data;
            all.extend(data.messages);
            cursor = data.response_metadata.next_cursor;
            if cursor.trim().is_empty() {
                break;
            }
        }
        Ok(all)
    }

    pub(crate) fn post_message(
        &self,
        channel_id: &str,
        thread_ts: Option<&str>,
        message: &str,
    ) -> Result<(), SlackError> {
        let mut body = json!({
            "channel": channel_id,
            "text": message,
        });
        if let Some(thread_ts) = thread_ts.filter(|v| !v.trim().is_empty()) {
            body["thread_ts"] = json!(thread_ts);
        }
        let envelope: SlackEnvelope<serde_json::Value> =
            self.post_json_with_token("chat.postMessage", &body, &self.bot_token)?;
        if !envelope.ok {
            return Err(SlackError::ApiResponse(
                envelope
                    .error
                    .unwrap_or_else(|| "chat.postMessage failed".to_string()),
            ));
        }
        Ok(())
    }
}
