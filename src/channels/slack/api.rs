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
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ConversationsListData {
    #[serde(default, alias = "channels")]
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
    #[serde(default)]
    pub(crate) reply_count: Option<u64>,
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
            .map_err(|e| Self::map_request_error(path, e))?;
        Self::decode_slack_response(path, response)
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
            .map_err(|e| Self::map_request_error(path, e))?;
        Self::decode_slack_response(path, response)
    }

    fn map_request_error(path: &str, error: ureq::Error) -> SlackError {
        match error {
            ureq::Error::Status(429, response) => {
                let retry_after_secs = response
                    .header("Retry-After")
                    .and_then(|raw| raw.trim().parse::<u64>().ok())
                    .filter(|value| *value > 0)
                    .unwrap_or(1);
                SlackError::RateLimited {
                    path: path.to_string(),
                    retry_after_secs,
                }
            }
            other => SlackError::ApiRequest(other.to_string()),
        }
    }

    fn decode_slack_response<T: for<'de> Deserialize<'de>>(
        path: &str,
        response: ureq::Response,
    ) -> Result<T, SlackError> {
        let value = response
            .into_json::<serde_json::Value>()
            .map_err(|e| SlackError::ApiRequest(e.to_string()))?;

        if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
            let error = value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown_error");
            let needed = value.get("needed").and_then(serde_json::Value::as_str);
            let provided = value.get("provided").and_then(serde_json::Value::as_str);

            let mut parts = vec![format!("{path} failed: {error}")];
            if let Some(needed) = needed {
                parts.push(format!("needed={needed}"));
            }
            if let Some(provided) = provided {
                parts.push(format!("provided={provided}"));
            }
            return Err(SlackError::ApiResponse(parts.join("; ")));
        }

        serde_json::from_value::<T>(value).map_err(|e| {
            SlackError::ApiRequest(format!("failed to parse {path} response JSON: {e}"))
        })
    }

    pub(crate) fn validate_connection(&self) -> Result<(), SlackError> {
        self.validate_auth()?;
        let _ = self.open_socket_connection_url()?;
        Ok(())
    }

    pub(crate) fn validate_auth(&self) -> Result<(), SlackError> {
        let auth: SlackEnvelope<EmptyData> =
            self.get_with_token("auth.test", &[], &self.bot_token)?;
        if !auth.ok {
            return Err(SlackError::ApiResponse(
                auth.error.unwrap_or_else(|| "auth.test failed".to_string()),
            ));
        }
        Ok(())
    }

    pub(crate) fn open_socket_connection_url(&self) -> Result<String, SlackError> {
        let conn: SlackEnvelope<OpenConnectionData> =
            self.post_json_with_token("apps.connections.open", &json!({}), &self.app_token)?;
        if !conn.ok {
            return Err(SlackError::ApiResponse(
                conn.error
                    .unwrap_or_else(|| "apps.connections.open failed".to_string()),
            ));
        }
        Ok(conn.data.url)
    }

    pub(crate) fn list_conversations(
        &self,
        include_im_conversations: bool,
    ) -> Result<Vec<ConversationSummary>, SlackError> {
        self.list_conversations_with_types(include_im_conversations, true)
    }

    fn list_conversations_with_types(
        &self,
        include_im_conversations: bool,
        allow_scope_fallback: bool,
    ) -> Result<Vec<ConversationSummary>, SlackError> {
        let mut all = Vec::new();
        let mut cursor = String::new();
        loop {
            let types = if include_im_conversations {
                "im,public_channel,private_channel"
            } else {
                "public_channel,private_channel"
            };
            let mut query = vec![("types", types.to_string()), ("limit", "200".to_string())];
            if !cursor.is_empty() {
                query.push(("cursor", cursor.clone()));
            }

            let envelope: SlackEnvelope<ConversationsListData> =
                match self.get_with_token("conversations.list", &query, &self.bot_token) {
                    Ok(envelope) => envelope,
                    Err(SlackError::ApiResponse(message))
                        if allow_scope_fallback
                            && include_im_conversations
                            && message.contains("missing_scope")
                            && message.contains("im:read") =>
                    {
                        return self.list_conversations_with_types(false, false);
                    }
                    Err(err) => return Err(err),
                };
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

    pub(crate) fn conversation_replies(
        &self,
        conversation_id: &str,
        thread_ts: &str,
        oldest: Option<&str>,
    ) -> Result<Vec<SlackMessage>, SlackError> {
        let mut all = Vec::new();
        let mut cursor = String::new();
        loop {
            let mut query = vec![
                ("channel", conversation_id.to_string()),
                ("ts", thread_ts.to_string()),
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
                self.get_with_token("conversations.replies", &query, &self.bot_token)?;
            if !envelope.ok {
                return Err(SlackError::ApiResponse(
                    envelope
                        .error
                        .unwrap_or_else(|| "conversations.replies failed".to_string()),
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
