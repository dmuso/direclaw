use crate::config::OrchestratorConfig;
use crate::orchestration::function_registry::FunctionRegistry;
use crate::orchestration::selector::{
    FunctionArgSchema, FunctionArgType, SelectorAction, SelectorRequest, SelectorResult,
    SelectorStatus,
};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LexicalRoutingConfig {
    pub min_confidence: f32,
    pub exact_tag_boost: f32,
    pub exact_token_boost: f32,
    pub negation_penalty: f32,
}

impl LexicalRoutingConfig {
    pub fn balanced() -> Self {
        Self {
            min_confidence: 0.70,
            exact_tag_boost: 1.8,
            exact_token_boost: 1.0,
            negation_penalty: 3.0,
        }
    }

    pub fn high_precision() -> Self {
        Self {
            min_confidence: 0.85,
            ..Self::balanced()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexicalSelectorSource {
    Lexical,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LexicalDecision {
    pub result: SelectorResult,
    pub confidence: f32,
    pub source: LexicalSelectorSource,
}

#[derive(Debug, Clone, PartialEq)]
struct Scored<T> {
    value: T,
    score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    WorkflowStart,
    WorkflowStatus,
    DiagnosticsInvestigate,
    CommandInvoke,
}

pub fn resolve_lexical_decision(
    request: &SelectorRequest,
    orchestrator: &OrchestratorConfig,
    _functions: &FunctionRegistry,
    config: &LexicalRoutingConfig,
) -> Option<LexicalDecision> {
    let query_tokens = tokenize_with_positions(&request.user_message);
    if query_tokens.is_empty() {
        return None;
    }

    let workflow_scores = score_workflows(orchestrator, &query_tokens, config);
    let function_scores = score_functions(request, &query_tokens, config);

    let status_score = score_intent(&query_tokens, STATUS_INTENT);
    let diagnostics_score = score_intent(&query_tokens, DIAGNOSTICS_INTENT);
    let command_intent_score = score_intent(&query_tokens, COMMAND_INTENT);

    let best_workflow = top_score(&workflow_scores)?;
    let best_function = top_score(&function_scores);

    let action_scores = [
        Scored {
            value: Intent::WorkflowStart,
            score: best_workflow.score,
        },
        Scored {
            value: Intent::WorkflowStatus,
            score: status_score,
        },
        Scored {
            value: Intent::DiagnosticsInvestigate,
            score: diagnostics_score,
        },
        Scored {
            value: Intent::CommandInvoke,
            score: best_function
                .as_ref()
                .map(|f| f.score + command_intent_score)
                .unwrap_or(command_intent_score),
        },
    ];

    let best_action = action_scores
        .iter()
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?
        .clone();

    let runner_up = action_scores
        .iter()
        .filter(|candidate| candidate.value != best_action.value)
        .map(|candidate| candidate.score)
        .fold(0.0_f32, f32::max);
    let confidence = confidence(best_action.score, runner_up);
    if confidence < config.min_confidence {
        return None;
    }

    let (action, selected_workflow, diagnostics_scope, function_id, function_args, reason) =
        match best_action.value {
            Intent::WorkflowStart => (
                SelectorAction::WorkflowStart,
                Some(best_workflow.value.clone()),
                None,
                None,
                None,
                format!(
                    "selector_source=lexical intent=workflow_start score={:.2}",
                    best_action.score
                ),
            ),
            Intent::WorkflowStatus => (
                SelectorAction::WorkflowStatus,
                None,
                None,
                None,
                None,
                format!(
                    "selector_source=lexical intent=workflow_status score={:.2}",
                    best_action.score
                ),
            ),
            Intent::DiagnosticsInvestigate => (
                SelectorAction::DiagnosticsInvestigate,
                None,
                Some(Map::new()),
                None,
                None,
                format!(
                    "selector_source=lexical intent=diagnostics_investigate score={:.2}",
                    best_action.score
                ),
            ),
            Intent::CommandInvoke => {
                let selected = best_function?;
                let schema = request
                    .available_function_schemas
                    .iter()
                    .find(|schema| schema.function_id == selected.value)?;
                let args = extract_function_args(request, orchestrator, schema, &query_tokens)?;
                (
                    SelectorAction::CommandInvoke,
                    None,
                    None,
                    Some(selected.value.clone()),
                    Some(args),
                    format!(
                        "selector_source=lexical intent=command_invoke score={:.2}",
                        best_action.score
                    ),
                )
            }
        };

    Some(LexicalDecision {
        result: SelectorResult {
            selector_id: request.selector_id.clone(),
            status: SelectorStatus::Selected,
            action: Some(action),
            selected_workflow,
            diagnostics_scope,
            function_id,
            function_args,
            reason: Some(reason),
        },
        confidence,
        source: LexicalSelectorSource::Lexical,
    })
}

fn tokenize_with_positions(input: &str) -> Vec<(usize, String)> {
    input
        .to_ascii_lowercase()
        .replace('\'', "")
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .filter(|token| !token.is_empty())
        .enumerate()
        .map(|(index, token)| (index, token.to_string()))
        .collect()
}

fn bm25_score(query: &[String], doc: &[String]) -> f32 {
    if query.is_empty() || doc.is_empty() {
        return 0.0;
    }
    let mut term_counts = HashMap::<&str, usize>::new();
    for token in doc {
        *term_counts.entry(token.as_str()).or_default() += 1;
    }
    let doc_len = doc.len() as f32;
    let avg_doc_len = 16.0_f32;
    let k1 = 1.2_f32;
    let b = 0.75_f32;

    query
        .iter()
        .map(|term| {
            let tf = *term_counts.get(term.as_str()).unwrap_or(&0) as f32;
            if tf == 0.0 {
                return 0.0;
            }
            let idf = 1.0_f32;
            idf * ((tf * (k1 + 1.0)) / (tf + k1 * (1.0 - b + b * (doc_len / avg_doc_len))))
        })
        .sum()
}

fn score_workflows(
    orchestrator: &OrchestratorConfig,
    query_tokens: &[(usize, String)],
    config: &LexicalRoutingConfig,
) -> Vec<Scored<String>> {
    let query = query_tokens
        .iter()
        .map(|(_, token)| token.clone())
        .collect::<Vec<_>>();
    let query_set = query.iter().cloned().collect::<HashSet<_>>();

    orchestrator
        .workflows
        .iter()
        .map(|workflow| {
            let mut doc = tokenize_with_positions(&workflow.id)
                .into_iter()
                .map(|(_, token)| token)
                .collect::<Vec<_>>();
            doc.extend(
                tokenize_with_positions(&workflow.description)
                    .into_iter()
                    .map(|(_, token)| token),
            );
            doc.extend(
                workflow
                    .tags
                    .iter()
                    .map(|tag| tag.as_str().to_string())
                    .collect::<Vec<_>>(),
            );
            doc.extend(
                workflow
                    .steps
                    .iter()
                    .flat_map(|step| tokenize_with_positions(&step.id))
                    .map(|(_, token)| token)
                    .collect::<Vec<_>>(),
            );

            let mut score = bm25_score(&query, &doc);
            for tag in &workflow.tags {
                if query_set.contains(tag.as_str()) {
                    score += config.exact_tag_boost;
                }
            }

            let id_tokens = tokenize_with_positions(&workflow.id)
                .into_iter()
                .map(|(_, token)| token)
                .collect::<Vec<_>>();
            if id_tokens.iter().any(|token| query_set.contains(token)) {
                score += config.exact_token_boost;
            }

            let penalty = negation_penalty(query_tokens, &doc, config.negation_penalty);
            Scored {
                value: workflow.id.clone(),
                score: (score - penalty).max(0.0),
            }
        })
        .collect()
}

fn score_functions(
    request: &SelectorRequest,
    query_tokens: &[(usize, String)],
    config: &LexicalRoutingConfig,
) -> Vec<Scored<String>> {
    let query = query_tokens
        .iter()
        .map(|(_, token)| token.clone())
        .collect::<Vec<_>>();
    let query_set = query.iter().cloned().collect::<HashSet<_>>();
    let schemas = request
        .available_function_schemas
        .iter()
        .map(|schema| (schema.function_id.clone(), schema))
        .collect::<BTreeMap<_, _>>();

    request
        .available_functions
        .iter()
        .map(|function_id| {
            let mut doc = tokenize_with_positions(function_id)
                .into_iter()
                .map(|(_, token)| token)
                .collect::<Vec<_>>();
            if let Some(schema) = schemas.get(function_id) {
                doc.extend(
                    tokenize_with_positions(&schema.description)
                        .into_iter()
                        .map(|(_, token)| token),
                );
                doc.extend(
                    schema
                        .args
                        .keys()
                        .flat_map(|arg| tokenize_with_positions(arg))
                        .map(|(_, token)| token),
                );
            }
            let mut score = bm25_score(&query, &doc);
            let id_tokens = tokenize_with_positions(function_id)
                .into_iter()
                .map(|(_, token)| token)
                .collect::<Vec<_>>();
            if id_tokens.iter().any(|token| query_set.contains(token)) {
                score += config.exact_token_boost;
            }
            let penalty = negation_penalty(query_tokens, &doc, config.negation_penalty);
            Scored {
                value: function_id.clone(),
                score: (score - penalty).max(0.0),
            }
        })
        .collect()
}

fn score_intent(query_tokens: &[(usize, String)], intent_tokens: &[&str]) -> f32 {
    let query = query_tokens
        .iter()
        .map(|(_, token)| token.clone())
        .collect::<Vec<_>>();
    let doc = intent_tokens
        .iter()
        .map(|token| token.to_string())
        .collect::<Vec<_>>();
    bm25_score(&query, &doc)
}

fn top_score<T: Clone>(scores: &[Scored<T>]) -> Option<Scored<T>> {
    scores
        .iter()
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .cloned()
}

fn confidence(top: f32, runner_up: f32) -> f32 {
    if top <= 0.0 {
        return 0.0;
    }
    let base = top / (top + 2.0);
    let margin = ((top - runner_up).max(0.0) / (top + 1.0)).min(1.0);
    (base + (0.25 * margin)).min(1.0)
}

fn negation_penalty(query_tokens: &[(usize, String)], doc: &[String], weight: f32) -> f32 {
    let doc_set = doc.iter().collect::<HashSet<_>>();
    let mut penalty = 0.0_f32;
    for (idx, token) in query_tokens {
        if !NEGATIONS.contains(&token.as_str()) {
            continue;
        }
        for (other_idx, other_token) in query_tokens {
            if *other_idx > *idx
                && (*other_idx - *idx) <= 3
                && doc_set.contains(other_token)
                && !NEGATIONS.contains(&other_token.as_str())
            {
                penalty += weight;
            }
        }
    }
    penalty
}

fn extract_function_args(
    request: &SelectorRequest,
    orchestrator: &OrchestratorConfig,
    schema: &crate::orchestration::selector::FunctionSchema,
    query_tokens: &[(usize, String)],
) -> Option<Map<String, Value>> {
    let mut args = Map::new();
    let run_id_token = query_tokens
        .iter()
        .map(|(_, token)| token)
        .find(|token| token.starts_with("run-"))
        .cloned();

    for (arg_name, arg_schema) in &schema.args {
        let value = infer_arg_value(
            arg_name,
            arg_schema,
            request,
            orchestrator,
            run_id_token.as_deref(),
            query_tokens,
        );
        match value {
            Some(value) => {
                args.insert(arg_name.clone(), value);
            }
            None if arg_schema.required => return None,
            None => {}
        }
    }
    Some(args)
}

fn infer_arg_value(
    arg_name: &str,
    arg_schema: &FunctionArgSchema,
    request: &SelectorRequest,
    orchestrator: &OrchestratorConfig,
    run_id_token: Option<&str>,
    query_tokens: &[(usize, String)],
) -> Option<Value> {
    match arg_name {
        "runId" => run_id_token.map(|value| Value::String(value.to_string())),
        "channelProfileId" => Some(Value::String(request.channel_profile_id.clone())),
        "orchestratorId" => Some(Value::String(orchestrator.id.clone())),
        "workflowId" => {
            if let Some(found) = request
                .available_workflows
                .iter()
                .find(|workflow_id| request.user_message.contains(workflow_id.as_str()))
            {
                return Some(Value::String(found.clone()));
            }
            if request.available_workflows.len() == 1 {
                return Some(Value::String(request.available_workflows[0].clone()));
            }
            None
        }
        _ => match arg_schema.arg_type {
            FunctionArgType::String => None,
            FunctionArgType::Boolean => {
                if query_tokens.iter().any(|(_, token)| token == "true") {
                    Some(Value::Bool(true))
                } else if query_tokens.iter().any(|(_, token)| token == "false") {
                    Some(Value::Bool(false))
                } else {
                    None
                }
            }
            FunctionArgType::Integer => query_tokens
                .iter()
                .find_map(|(_, token)| token.parse::<i64>().ok())
                .map(Value::from),
            FunctionArgType::Object => None,
        },
    }
}

const STATUS_INTENT: &[&str] = &[
    "status", "progress", "update", "latest", "state", "running", "current", "how", "far",
];
const DIAGNOSTICS_INTENT: &[&str] = &[
    "why",
    "failed",
    "fail",
    "error",
    "investigate",
    "diagnostics",
    "diagnose",
    "root",
    "cause",
];
const COMMAND_INTENT: &[&str] = &[
    "list",
    "show",
    "set",
    "add",
    "remove",
    "cancel",
    "start",
    "stop",
    "restart",
    "run",
    "workflow",
    "orchestrator",
    "agent",
    "daemon",
    "channel",
];
const NEGATIONS: &[&str] = &["not", "dont", "don", "no", "without"];
