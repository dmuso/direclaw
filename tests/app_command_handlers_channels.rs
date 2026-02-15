use direclaw::app::command_handlers::channels::{cmd_channels, cmd_send};

#[test]
fn channels_handler_module_exposes_commands() {
    let _ = cmd_channels as fn(&[String]) -> Result<String, String>;
    let _ = cmd_send as fn(&[String]) -> Result<String, String>;
}
