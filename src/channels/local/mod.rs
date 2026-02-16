mod session;
mod tui;

use crate::config::Settings;
use std::path::Path;

pub fn run_local_chat_session(
    state_root: &Path,
    settings: &Settings,
    profile_id: &str,
) -> Result<String, String> {
    let session = session::create_local_chat_session(state_root, settings, profile_id)?;
    let conversation_id = session.conversation_id.clone();
    tui::run_local_chat_session_tui(session)?;
    Ok(format!("chat ended\nconversation_id={conversation_id}"))
}
