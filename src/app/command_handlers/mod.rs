use crate::app::cli::{help_text, parse_cli_verb, CliVerb};
use crate::app::command_catalog::{function_def, FunctionArgTypeDef, FunctionDef};
use crate::app::command_dispatch::{
    execute_function_invocation_with_executor, FunctionExecutionContext,
};
use crate::app::command_support::{ensure_runtime_root, load_settings};
use crate::orchestration::error::OrchestratorError;
use crate::orchestration::run_store::WorkflowRunStore;
use serde_json::{Map, Value};

type ParsedCatalogInvocation = Result<(String, Map<String, Value>), String>;

pub mod agents;
pub mod attach;
pub mod auth;
pub mod channel_profiles;
pub mod channels;
pub mod chat;
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

    if let Some(result) = try_execute_selector_cli_alias(&args) {
        return result;
    }

    run_cli_native(args)
}

fn run_cli_native(args: Vec<String>) -> Result<String, String> {
    match parse_cli_verb(args[0].as_str()) {
        CliVerb::Setup => crate::setup::actions::cmd_setup(),
        CliVerb::Start => daemon::cmd_start(),
        CliVerb::Stop => daemon::cmd_stop(),
        CliVerb::Restart => daemon::cmd_restart(),
        CliVerb::Status => daemon::cmd_status(),
        CliVerb::Logs => daemon::cmd_logs(),
        CliVerb::Send => channels::cmd_send(&args[1..]),
        CliVerb::Update => update::cmd_update(&args[1..]),
        CliVerb::Doctor => doctor::cmd_doctor(),
        CliVerb::Attach => attach::cmd_attach(),
        CliVerb::Chat => chat::cmd_chat(&args[1..]),
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

fn try_execute_selector_cli_alias(args: &[String]) -> Option<Result<String, String>> {
    let invocation = parse_catalog_cli_invocation(args)?;
    let (function_id, function_args) = match invocation {
        Ok(invocation) => invocation,
        Err(error) => return Some(Err(error)),
    };

    let settings = load_settings().ok();
    let run_store = ensure_runtime_root()
        .ok()
        .map(|paths| WorkflowRunStore::new(&paths.root));
    let context = FunctionExecutionContext {
        run_store: run_store.as_ref(),
        settings: settings.as_ref(),
    };

    Some(
        execute_function_invocation_with_executor(
            function_id.as_str(),
            &function_args,
            context,
            run_cli_native,
        )
        .map_err(|error| error.to_string())
        .and_then(render_function_result),
    )
}

fn parse_catalog_cli_invocation(args: &[String]) -> Option<ParsedCatalogInvocation> {
    let head = args.first()?;
    let def = function_def(head)?;
    Some(parse_catalog_function_args(def, &args[1..]).map(|parsed| (head.clone(), parsed)))
}

fn parse_catalog_function_args(
    def: &FunctionDef,
    positional: &[String],
) -> Result<Map<String, Value>, String> {
    let required_count = def.args.iter().filter(|arg| arg.required).count();
    if positional.len() < required_count {
        return Err(format!(
            "invalid arguments for `{}`: expected at least {} positional argument(s)",
            def.function_id, required_count
        ));
    }
    let allows_joined_tail = matches!(
        def.args.last().map(|arg| arg.arg_type),
        Some(FunctionArgTypeDef::String)
    );
    if positional.len() > def.args.len() && !allows_joined_tail {
        return Err(format!(
            "invalid arguments for `{}`: expected at most {} positional argument(s)",
            def.function_id,
            def.args.len()
        ));
    }

    let mut mapped = Map::new();
    for (index, arg_def) in def.args.iter().enumerate() {
        let Some(raw) = positional.get(index) else {
            continue;
        };
        let raw_value = if allows_joined_tail && index == def.args.len() - 1 {
            positional[index..].join(" ")
        } else {
            raw.clone()
        };
        let value = parse_typed_cli_value(arg_def.arg_type, &raw_value).map_err(|error| {
            format!(
                "invalid argument `{}` for `{}`: {error}",
                arg_def.name, def.function_id
            )
        })?;
        mapped.insert(arg_def.name.to_string(), value);
        if allows_joined_tail && index == def.args.len() - 1 {
            break;
        }
    }

    Ok(mapped)
}

fn parse_typed_cli_value(arg_type: FunctionArgTypeDef, raw: &str) -> Result<Value, String> {
    match arg_type {
        FunctionArgTypeDef::String => Ok(Value::String(raw.to_string())),
        FunctionArgTypeDef::Boolean => match raw {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err("expected `true` or `false`".to_string()),
        },
        FunctionArgTypeDef::Integer => {
            let parsed: i64 = raw
                .parse()
                .map_err(|_| "expected signed integer".to_string())?;
            Ok(Value::Number(parsed.into()))
        }
        FunctionArgTypeDef::Object => {
            let parsed: Value =
                serde_json::from_str(raw).map_err(|_| "expected JSON object".to_string())?;
            if !parsed.is_object() {
                return Err("expected JSON object".to_string());
            }
            Ok(parsed)
        }
    }
}

fn render_function_result(value: Value) -> Result<String, String> {
    if let Some(output) = value
        .as_object()
        .and_then(|obj| obj.get("output"))
        .and_then(Value::as_str)
    {
        return Ok(output.to_string());
    }

    serde_json::to_string_pretty(&value)
        .map_err(|error| format!("failed to format function result: {error}"))
}
