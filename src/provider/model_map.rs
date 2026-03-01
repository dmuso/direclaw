use crate::provider::ProviderError;

pub fn resolve_anthropic_model(model: &str) -> Result<String, ProviderError> {
    match model.trim() {
        "sonnet" => Ok("claude-sonnet-4-5".to_string()),
        "opus" => Ok("claude-opus-4-6".to_string()),
        "haiku" => Ok("claude-haiku-4-5".to_string()),
        "claude-sonnet-4-5" => Ok("claude-sonnet-4-5".to_string()),
        "claude-opus-4-6" => Ok("claude-opus-4-6".to_string()),
        "claude-haiku-4-5" => Ok("claude-haiku-4-5".to_string()),
        other => Err(ProviderError::UnsupportedAnthropicModel(other.to_string())),
    }
}
