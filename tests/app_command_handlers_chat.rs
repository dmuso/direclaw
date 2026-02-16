use direclaw::app::command_handlers::chat::cmd_chat;

#[test]
fn chat_handler_module_exposes_command() {
    let _ = cmd_chat as fn(&[String]) -> Result<String, String>;
}
