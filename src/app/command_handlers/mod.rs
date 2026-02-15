use crate::app::cli::{help_text, parse_cli_verb, CliVerb};
use crate::app::command_dispatch::{
    execute_function_invocation_with_executor, FunctionExecutionContext,
};
use crate::orchestration::error::OrchestratorError;
use serde_json::{Map, Value};

pub mod agents;
pub mod attach;
pub mod auth;
pub mod channel_profiles;
pub mod channels;
pub mod daemon;
pub mod doctor;
pub mod orchestrators;
pub mod provider;
pub mod update;
pub mod workflows;

pub fn execute_function_invocation(
    function_id: &str,
    args: &Map<String, Value>,
    context: FunctionExecutionContext<'_>,
) -> Result<Value, OrchestratorError> {
    execute_function_invocation_with_executor(function_id, args, context, run_cli)
}

pub fn run_cli(args: Vec<String>) -> Result<String, String> {
    if args.is_empty() {
        return Ok(help_text());
    }

    match parse_cli_verb(args[0].as_str()) {
        CliVerb::Setup => crate::tui::setup::cmd_setup(),
        CliVerb::Start => daemon::cmd_start(),
        CliVerb::Stop => daemon::cmd_stop(),
        CliVerb::Restart => daemon::cmd_restart(),
        CliVerb::Status => daemon::cmd_status(),
        CliVerb::Logs => daemon::cmd_logs(),
        CliVerb::Send => channels::cmd_send(&args[1..]),
        CliVerb::Update => update::cmd_update(&args[1..]),
        CliVerb::Doctor => doctor::cmd_doctor(),
        CliVerb::Attach => attach::cmd_attach(),
        CliVerb::Channels => channels::cmd_channels(&args[1..]),
        CliVerb::Provider => provider::cmd_provider(&args[1..]),
        CliVerb::Model => provider::cmd_model(&args[1..]),
        CliVerb::Agent => agents::cmd_orchestrator_agent(&args[1..]),
        CliVerb::Orchestrator => orchestrators::cmd_orchestrator(&args[1..]),
        CliVerb::OrchestratorAgent => agents::cmd_orchestrator_agent(&args[1..]),
        CliVerb::Workflow => workflows::cmd_workflow(&args[1..]),
        CliVerb::ChannelProfile => channel_profiles::cmd_channel_profile(&args[1..]),
        CliVerb::Auth => auth::cmd_auth(&args[1..]),
        CliVerb::Supervisor => daemon::cmd_supervisor(&args[1..]),
        CliVerb::Unknown => Err(format!("unknown command `{}`", args[0])),
    }
}
