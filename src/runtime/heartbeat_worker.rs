use crate::config::{load_orchestrator_config, Settings};
use crate::orchestration::workspace_access::resolve_agent_workspace_root;
use crate::queue::{sorted_outgoing_paths, IncomingMessage, OutgoingMessage, QueuePaths};
use crate::runtime::{append_runtime_log, StatePaths};
use std::fs;
use std::path::Path;
use std::time::Duration;

pub fn configured_heartbeat_interval(settings: &Settings) -> Option<Duration> {
    let seconds = settings.monitoring.heartbeat_interval.unwrap_or(3600);
    if seconds == 0 {
        None
    } else {
        Some(Duration::from_secs(seconds))
    }
}

pub fn resolve_heartbeat_prompt(
    agent_dir: &Path,
    orchestrator_id: &str,
    agent_id: &str,
) -> Result<String, String> {
    let prompt_path = agent_dir.join("heartbeat.md");
    match fs::read_to_string(&prompt_path) {
        Ok(body) => Ok(body.trim().to_string()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(format!(
            "Fallback heartbeat prompt for orchestrator `{orchestrator_id}` agent `{agent_id}`: respond with short runtime health status."
        )),
        Err(err) => Err(format!(
            "failed to read {}: {err}",
            prompt_path.display()
        )),
    }
}

pub fn build_heartbeat_incoming_message(
    orchestrator_id: &str,
    agent_id: &str,
    prompt: &str,
    tick_at: i64,
) -> Result<IncomingMessage, String> {
    if orchestrator_id.trim().is_empty() {
        return Err("heartbeat orchestrator id must be non-empty".to_string());
    }
    if agent_id.trim().is_empty() {
        return Err("heartbeat agent id must be non-empty".to_string());
    }
    let safe_orchestrator_id = sanitize_component(orchestrator_id);
    let safe_agent_id = sanitize_component(agent_id);
    let correlation = format!("hb:{safe_orchestrator_id}:{safe_agent_id}");
    Ok(IncomingMessage {
        channel: "heartbeat".to_string(),
        channel_profile_id: None,
        sender: format!("heartbeat:{orchestrator_id}"),
        sender_id: format!("heartbeat-{safe_agent_id}"),
        message: prompt.to_string(),
        timestamp: tick_at,
        message_id: format!("heartbeat-{safe_orchestrator_id}-{safe_agent_id}-{tick_at}"),
        conversation_id: Some(correlation.clone()),
        files: Vec::new(),
        workflow_run_id: Some(correlation),
        workflow_step_id: Some("heartbeat_worker_check".to_string()),
    })
}

pub fn match_heartbeat_responses(
    queue: &QueuePaths,
    _orchestrator_id: &str,
    _agent_id: &str,
    _heartbeat_message_id: &str,
    correlation_id: &str,
) -> Result<Option<String>, String> {
    let outgoing = sorted_outgoing_paths(queue)
        .map_err(|err| format!("failed to read {}: {err}", queue.outgoing.display()))?;

    for path in outgoing {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(payload) = serde_json::from_str::<OutgoingMessage>(&raw) else {
            continue;
        };
        if payload.channel != "heartbeat" {
            continue;
        }
        if payload.conversation_id.as_deref() != Some(correlation_id) {
            continue;
        }
        return Ok(Some(snippet(&payload.message, 200)));
    }

    Ok(None)
}

pub fn tick_heartbeat_worker(state_root: &Path, settings: &Settings) -> Result<(), String> {
    if std::env::var("DIRECLAW_FAIL_HEARTBEAT_TICK")
        .ok()
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return Err("fault injection requested for heartbeat tick".to_string());
    }

    let tick_at = heartbeat_tick_timestamp();
    let paths = StatePaths::new(state_root);
    let mut failures = Vec::new();

    for orchestrator_id in settings.orchestrators.keys() {
        let orchestrator = match load_orchestrator_config(settings, orchestrator_id) {
            Ok(cfg) => cfg,
            Err(err) => {
                failures.push(format!(
                    "orchestrator `{orchestrator_id}` heartbeat setup failed: {err}"
                ));
                continue;
            }
        };
        let runtime_root = match settings.resolve_orchestrator_runtime_root(orchestrator_id) {
            Ok(path) => path,
            Err(err) => {
                failures.push(format!(
                    "orchestrator `{orchestrator_id}` runtime root resolution failed: {err}"
                ));
                continue;
            }
        };
        let queue = QueuePaths::from_state_root(&runtime_root);
        if let Err(err) = fs::create_dir_all(&queue.incoming) {
            failures.push(format!(
                "failed to create {}: {err}",
                queue.incoming.display()
            ));
            continue;
        }
        if let Err(err) = fs::create_dir_all(&queue.outgoing) {
            failures.push(format!(
                "failed to create {}: {err}",
                queue.outgoing.display()
            ));
            continue;
        }

        let private_workspace = match settings.resolve_private_workspace(orchestrator_id) {
            Ok(path) => path,
            Err(err) => {
                failures.push(format!(
                    "failed to resolve private workspace for `{orchestrator_id}`: {err}"
                ));
                continue;
            }
        };

        for (agent_id, agent) in &orchestrator.agents {
            let agent_dir = resolve_agent_workspace_root(&private_workspace, agent_id, agent);
            let prompt = match resolve_heartbeat_prompt(&agent_dir, orchestrator_id, agent_id) {
                Ok(prompt) => prompt,
                Err(err) => {
                    failures.push(err);
                    continue;
                }
            };
            let message =
                match build_heartbeat_incoming_message(orchestrator_id, agent_id, &prompt, tick_at)
                {
                    Ok(message) => message,
                    Err(err) => {
                        failures.push(err);
                        continue;
                    }
                };
            let queue_path = queue.incoming.join(format!("{}.json", message.message_id));
            let body = match serde_json::to_vec_pretty(&message) {
                Ok(body) => body,
                Err(err) => {
                    failures.push(format!(
                        "failed to encode heartbeat message `{}`: {err}",
                        message.message_id
                    ));
                    continue;
                }
            };
            if let Err(err) = fs::write(&queue_path, body) {
                failures.push(format!("failed to write {}: {err}", queue_path.display()));
                continue;
            }

            let correlation = message.conversation_id.clone().unwrap_or_default();
            match match_heartbeat_responses(
                &queue,
                orchestrator_id,
                agent_id,
                &message.message_id,
                &correlation,
            ) {
                Ok(Some(response)) => append_runtime_log(
                    &paths,
                    "info",
                    "heartbeat.response.matched",
                    &format!(
                        "orchestrator={orchestrator_id} agent={agent_id} message_id={} snippet={}",
                        message.message_id, response
                    ),
                ),
                Ok(None) => append_runtime_log(
                    &paths,
                    "info",
                    "heartbeat.response.missing",
                    &format!(
                        "orchestrator={orchestrator_id} agent={agent_id} message_id={}",
                        message.message_id
                    ),
                ),
                Err(err) => failures.push(err),
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn heartbeat_tick_timestamp() -> i64 {
    std::env::var("DIRECLAW_HEARTBEAT_TICK_AT")
        .ok()
        .and_then(|raw| raw.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(crate::runtime::now_secs)
}

fn sanitize_component(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn snippet(text: &str, max_chars: usize) -> String {
    let mut short = String::new();
    short.extend(text.chars().take(max_chars));
    if text.chars().count() > max_chars {
        short.push_str("...");
    }
    short
}
