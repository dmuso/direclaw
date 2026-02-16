use crate::app::command_support::{ensure_runtime_root, load_settings};
use crate::channels::local::run_local_chat_session;

pub fn cmd_chat(args: &[String]) -> Result<String, String> {
    if args.len() != 1 {
        return Err("usage: chat <channel_profile_id>".to_string());
    }

    let settings = load_settings()?;
    let paths = ensure_runtime_root()?;
    run_local_chat_session(&paths.root, &settings, &args[0])
}
