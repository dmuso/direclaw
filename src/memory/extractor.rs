use super::domain::{
    MemoryCapturedBy, MemoryEdge, MemoryEdgeType, MemoryNode, MemoryNodeType, MemorySource,
    MemorySourceType, MemoryStatus,
};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtractedMemory {
    pub nodes: Vec<MemoryNode>,
    pub edges: Vec<MemoryEdge>,
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryExtractionError {
    #[error("unsupported file type `{extension}`")]
    UnsupportedFileType { extension: String },
    #[error("failed to parse ingest json: {0}")]
    InvalidJson(String),
    #[error("invalid source path for extraction")]
    InvalidSourcePath,
    #[error(
        "json memory `{memory_id}` orchestrator scope mismatch: expected {expected}, got {actual}"
    )]
    JsonScopeMismatch {
        memory_id: String,
        expected: String,
        actual: String,
    },
    #[error("json memory `{memory_id}` requires non-empty content")]
    EmptyContent { memory_id: String },
    #[error("json memory `{memory_id}` requires non-empty summary")]
    EmptySummary { memory_id: String },
}

pub fn extract_candidates_from_ingest_file(
    orchestrator_id: &str,
    canonical_source_path: &Path,
    bytes: &[u8],
) -> Result<ExtractedMemory, MemoryExtractionError> {
    if !canonical_source_path.is_absolute() {
        return Err(MemoryExtractionError::InvalidSourcePath);
    }

    let extension = canonical_source_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "txt" => extract_plain_text(
            orchestrator_id,
            canonical_source_path,
            bytes,
            MemoryNodeType::Observation,
        ),
        "md" => extract_plain_text(
            orchestrator_id,
            canonical_source_path,
            bytes,
            MemoryNodeType::Observation,
        ),
        "json" => extract_json(orchestrator_id, canonical_source_path, bytes),
        other => Err(MemoryExtractionError::UnsupportedFileType {
            extension: other.to_string(),
        }),
    }
}

fn extract_plain_text(
    orchestrator_id: &str,
    canonical_source_path: &Path,
    bytes: &[u8],
    node_type: MemoryNodeType,
) -> Result<ExtractedMemory, MemoryExtractionError> {
    let raw = String::from_utf8_lossy(bytes);
    let content = normalize_text(&raw);
    let summary = make_summary(&content);
    let now = now_secs();

    let node = MemoryNode {
        memory_id: derive_memory_id(canonical_source_path, bytes, 0),
        orchestrator_id: orchestrator_id.to_string(),
        node_type,
        importance: 50,
        content,
        summary,
        confidence: 0.6,
        source: ingest_source(canonical_source_path),
        status: MemoryStatus::Active,
        created_at: now,
        updated_at: now,
    };

    Ok(ExtractedMemory {
        nodes: vec![node],
        edges: Vec::new(),
    })
}

fn extract_json(
    orchestrator_id: &str,
    canonical_source_path: &Path,
    bytes: &[u8],
) -> Result<ExtractedMemory, MemoryExtractionError> {
    let parsed: JsonExtractPayload = serde_json::from_slice(bytes)
        .map_err(|err| MemoryExtractionError::InvalidJson(err.to_string()))?;
    let now = now_secs();

    let mut nodes = Vec::with_capacity(parsed.memories.len());
    for memory in parsed.memories {
        if let Some(json_orchestrator_id) = memory.orchestrator_id.as_ref() {
            if json_orchestrator_id != orchestrator_id {
                return Err(MemoryExtractionError::JsonScopeMismatch {
                    memory_id: memory.memory_id.clone(),
                    expected: orchestrator_id.to_string(),
                    actual: json_orchestrator_id.clone(),
                });
            }
        }

        let content = normalize_text(&memory.content);
        if content.is_empty() {
            return Err(MemoryExtractionError::EmptyContent {
                memory_id: memory.memory_id,
            });
        }

        let summary = normalize_text(&memory.summary);
        if summary.is_empty() {
            return Err(MemoryExtractionError::EmptySummary {
                memory_id: memory.memory_id,
            });
        }

        nodes.push(MemoryNode {
            memory_id: memory.memory_id,
            orchestrator_id: orchestrator_id.to_string(),
            node_type: memory.node_type,
            importance: memory.importance,
            content,
            summary,
            confidence: memory.confidence,
            source: ingest_source(canonical_source_path),
            status: memory.status,
            created_at: memory.created_at.unwrap_or(now),
            updated_at: memory.updated_at.unwrap_or(now),
        });
    }

    let edges = parsed
        .edges
        .into_iter()
        .map(|edge| MemoryEdge {
            edge_id: edge.edge_id,
            from_memory_id: edge.from_memory_id,
            to_memory_id: edge.to_memory_id,
            edge_type: edge.edge_type,
            weight: edge.weight,
            created_at: edge.created_at.unwrap_or(now),
            reason: edge.reason.map(|value| normalize_text(&value)),
        })
        .collect();

    Ok(ExtractedMemory { nodes, edges })
}

fn normalize_text(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn make_summary(content: &str) -> String {
    let mut summary = String::new();
    for chunk in content.split_whitespace() {
        if summary.is_empty() {
            summary.push_str(chunk);
        } else if summary.len() + chunk.len() < 120 {
            summary.push(' ');
            summary.push_str(chunk);
        } else {
            break;
        }
    }

    if summary.is_empty() {
        "(empty)".to_string()
    } else {
        summary
    }
}

fn derive_memory_id(path: &Path, bytes: &[u8], index: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_os_str().as_encoded_bytes());
    hasher.update([0]);
    hasher.update(bytes);
    hasher.update([0]);
    hasher.update(index.to_string().as_bytes());
    let digest = hasher.finalize();
    format!("ingest-{}", to_hex(&digest[..12]))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn ingest_source(canonical_source_path: &Path) -> MemorySource {
    MemorySource {
        source_type: MemorySourceType::IngestFile,
        source_path: Some(canonical_source_path.to_path_buf()),
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::Extractor,
    }
}

fn now_secs() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(now.as_secs()).unwrap_or(i64::MAX)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonExtractPayload {
    #[serde(default)]
    memories: Vec<JsonMemoryNode>,
    #[serde(default)]
    edges: Vec<JsonMemoryEdge>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonMemoryNode {
    memory_id: String,
    #[serde(default)]
    orchestrator_id: Option<String>,
    #[serde(rename = "type")]
    node_type: MemoryNodeType,
    importance: u8,
    content: String,
    summary: String,
    confidence: f32,
    status: MemoryStatus,
    #[serde(default)]
    created_at: Option<i64>,
    #[serde(default)]
    updated_at: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonMemoryEdge {
    edge_id: String,
    from_memory_id: String,
    to_memory_id: String,
    edge_type: MemoryEdgeType,
    weight: f32,
    #[serde(default)]
    created_at: Option<i64>,
    #[serde(default)]
    reason: Option<String>,
}
