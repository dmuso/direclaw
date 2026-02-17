use super::domain::{
    MemoryCapturedBy, MemoryNode, MemoryNodeType, MemorySource, MemorySourceType, MemoryStatus,
};
use super::repository::{
    MemoryRepository, MemoryRepositoryError, MemorySourceRecord, PersistOutcome,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub fn persist_transcript_observation(
    repo: &MemoryRepository,
    orchestrator_id: &str,
    message_id: &str,
    conversation_id: &str,
    transcript_text: &str,
    captured_at: i64,
) -> Result<PersistOutcome, MemoryRepositoryError> {
    let content = normalize_text(transcript_text);
    if content.is_empty() {
        return Ok(PersistOutcome::DuplicateSource);
    }

    let source = MemorySource {
        source_type: MemorySourceType::ChannelTranscript,
        source_path: None,
        conversation_id: Some(conversation_id.to_string()),
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::User,
    };
    let node = MemoryNode {
        memory_id: format!("transcript-{message_id}"),
        orchestrator_id: orchestrator_id.to_string(),
        node_type: MemoryNodeType::Observation,
        importance: 45,
        content: content.clone(),
        summary: summarize(&content, 120),
        confidence: 0.7,
        source,
        status: MemoryStatus::Active,
        created_at: captured_at,
        updated_at: captured_at,
    };
    let source_record = MemorySourceRecord {
        orchestrator_id: orchestrator_id.to_string(),
        idempotency_key: format!("channel_transcript:{message_id}"),
        source_type: MemorySourceType::ChannelTranscript,
        source_path: None,
        conversation_id: Some(conversation_id.to_string()),
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::User,
    };

    repo.upsert_nodes_and_edges(&source_record, &[node], &[])
}

pub fn persist_workflow_output_memories(
    repo: &MemoryRepository,
    input: &WorkflowOutputWriteback<'_>,
) -> Result<PersistOutcome, MemoryRepositoryError> {
    let mut nodes = Vec::new();
    for (key, value) in input.outputs {
        let content = render_output_value(value);
        if content.trim().is_empty() {
            continue;
        }

        let source_path = input.output_files.get(key).and_then(|path| {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                Some(path)
            } else {
                None
            }
        });
        let source = MemorySource {
            source_type: MemorySourceType::WorkflowOutput,
            source_path,
            conversation_id: input.conversation_id.map(|v| v.to_string()),
            workflow_run_id: Some(input.run_id.to_string()),
            step_id: Some(input.step_id.to_string()),
            captured_by: MemoryCapturedBy::System,
        };
        let node_type = node_type_for_output_key(key);
        nodes.push(MemoryNode {
            memory_id: format!(
                "workflow-{}-{}-{}",
                input.run_id,
                input.step_id,
                sanitize_id_component(key)
            ),
            orchestrator_id: input.orchestrator_id.to_string(),
            node_type,
            importance: importance_for_node_type(node_type),
            summary: summarize(&content, 120),
            content,
            confidence: 0.75,
            source,
            status: MemoryStatus::Active,
            created_at: input.captured_at,
            updated_at: input.captured_at,
        });
    }

    if nodes.is_empty() {
        return Ok(PersistOutcome::DuplicateSource);
    }

    let source_record = MemorySourceRecord {
        orchestrator_id: input.orchestrator_id.to_string(),
        idempotency_key: format!(
            "workflow_output:{}:{}:{}",
            input.run_id, input.step_id, input.attempt
        ),
        source_type: MemorySourceType::WorkflowOutput,
        source_path: None,
        conversation_id: input.conversation_id.map(|v| v.to_string()),
        workflow_run_id: Some(input.run_id.to_string()),
        step_id: Some(input.step_id.to_string()),
        captured_by: MemoryCapturedBy::System,
    };
    repo.upsert_nodes_and_edges(&source_record, &nodes, &[])
}

pub struct WorkflowOutputWriteback<'a> {
    pub orchestrator_id: &'a str,
    pub run_id: &'a str,
    pub step_id: &'a str,
    pub attempt: u32,
    pub conversation_id: Option<&'a str>,
    pub outputs: &'a Map<String, Value>,
    pub output_files: &'a BTreeMap<String, String>,
    pub captured_at: i64,
}

fn node_type_for_output_key(key: &str) -> MemoryNodeType {
    let lowered = key.to_ascii_lowercase();
    if lowered.contains("decision") {
        MemoryNodeType::Decision
    } else if lowered.contains("todo") {
        MemoryNodeType::Todo
    } else if lowered.contains("goal") {
        MemoryNodeType::Goal
    } else if lowered.contains("preference") {
        MemoryNodeType::Preference
    } else {
        MemoryNodeType::Fact
    }
}

fn importance_for_node_type(node_type: MemoryNodeType) -> u8 {
    match node_type {
        MemoryNodeType::Decision => 75,
        MemoryNodeType::Goal => 70,
        MemoryNodeType::Todo => 65,
        MemoryNodeType::Preference => 60,
        MemoryNodeType::Fact => 55,
        MemoryNodeType::Identity | MemoryNodeType::Event | MemoryNodeType::Observation => 50,
    }
}

fn normalize_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn summarize(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    content.chars().take(max_chars).collect()
}

fn render_output_value(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return normalize_text(text);
    }
    serde_json::to_string(value).unwrap_or_default()
}

fn sanitize_id_component(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}
