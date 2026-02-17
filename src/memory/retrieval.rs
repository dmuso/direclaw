use super::domain::{
    validate_confidence, validate_edge_weight, MemoryCapturedBy, MemoryEdge, MemoryEdgeType,
    MemoryNode, MemoryNodeType, MemorySource, MemorySourceType, MemoryStatus,
};
use super::logging::append_memory_event;
use super::repository::{MemoryRepository, MemoryRepositoryError};
use crate::orchestration::workspace_access::{enforce_workspace_access, WorkspaceAccessContext};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum MemoryRecallError {
    #[error("memory repository error: {0}")]
    Repository(#[from] MemoryRepositoryError),
    #[error("cross-orchestrator recall denied: requested={requested} available={available}")]
    CrossOrchestratorDenied {
        requested: String,
        available: String,
    },
    #[error("memory source path access denied: {path}")]
    SourcePathAccessDenied { path: String },
    #[error("sqlite query failed: {source}")]
    Sql {
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to append memory log at {path}: {source}")]
    LogWrite {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid node type `{value}` in database")]
    InvalidNodeType { value: String },
    #[error("invalid memory status `{value}` in database")]
    InvalidStatus { value: String },
    #[error("invalid source type `{value}` in database")]
    InvalidSourceType { value: String },
    #[error("invalid captured_by `{value}` in database")]
    InvalidCapturedBy { value: String },
    #[error("invalid edge type `{value}` in database")]
    InvalidEdgeType { value: String },
    #[error("invalid importance `{value}` in database for memory `{memory_id}`")]
    InvalidImportance { memory_id: String, value: i64 },
    #[error("invalid confidence `{value}` in database for memory `{memory_id}`")]
    InvalidConfidence { memory_id: String, value: f32 },
    #[error("invalid edge weight `{value}` in database for edge `{edge_id}`")]
    InvalidEdgeWeight { edge_id: String, value: f32 },
    #[error("failed to decode embedding for memory `{memory_id}`")]
    InvalidEmbedding { memory_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCitation {
    pub memory_id: String,
    pub source_type: String,
    #[serde(default)]
    pub source_path: Option<PathBuf>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub workflow_run_id: Option<String>,
    #[serde(default)]
    pub step_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProvenanceHandle {
    pub source_type: MemorySourceType,
    pub source_path: Option<PathBuf>,
    pub conversation_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub step_id: Option<String>,
    pub captured_by: MemoryCapturedBy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FullTextCandidate {
    pub memory: MemoryNode,
    pub provenance: MemoryProvenanceHandle,
    pub rank: usize,
    pub bm25_score: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorCandidate {
    pub memory: MemoryNode,
    pub provenance: MemoryProvenanceHandle,
    pub rank: usize,
    pub similarity: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VectorQueryOutcome {
    Ranked(Vec<VectorCandidate>),
    UnavailableMissingQueryEmbedding,
    UnavailableNoEmbeddings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridRecallResultMode {
    Hybrid,
    FullTextOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HybridRecallMemory {
    pub memory: MemoryNode,
    pub provenance: MemoryProvenanceHandle,
    pub citation: MemoryCitation,
    pub final_score: f64,
    pub unresolved_contradiction: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HybridRecallResult {
    pub mode: HybridRecallResultMode,
    pub memories: Vec<HybridRecallMemory>,
    pub edges: Vec<MemoryEdge>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HybridRecallRequest {
    pub requesting_orchestrator_id: String,
    pub conversation_id: Option<String>,
    pub query_text: String,
    pub query_embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecallOptions {
    pub top_n: usize,
    pub rrf_k: usize,
    pub top_k_text: usize,
    pub top_k_vector: usize,
    pub now_unix_seconds: i64,
}

impl Default for MemoryRecallOptions {
    fn default() -> Self {
        Self {
            top_n: 20,
            rrf_k: 60,
            top_k_text: 50,
            top_k_vector: 50,
            now_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        }
    }
}

pub fn query_full_text(
    repo: &MemoryRepository,
    query: &str,
    top_k: usize,
) -> Result<Vec<FullTextCandidate>, MemoryRecallError> {
    if query.trim().is_empty() || top_k == 0 {
        return Ok(Vec::new());
    }

    let connection = open_connection(repo)?;
    let mut statement = connection
        .prepare(
            "
            SELECT
                m.memory_id,
                m.orchestrator_id,
                m.node_type,
                m.importance,
                m.content,
                m.summary,
                m.confidence,
                m.status,
                m.source_type,
                m.source_path,
                m.conversation_id,
                m.workflow_run_id,
                m.step_id,
                m.captured_by,
                m.created_at,
                m.updated_at,
                bm25(memory_fts) AS bm25_score
            FROM memory_fts
            JOIN memories m
              ON m.orchestrator_id = memory_fts.orchestrator_id
             AND m.memory_id = memory_fts.memory_id
            WHERE memory_fts MATCH ?1
              AND m.orchestrator_id = ?2
              AND m.status != 'retracted'
            ORDER BY bm25_score ASC, m.memory_id ASC
            LIMIT ?3
            ",
        )
        .map_err(|source| MemoryRecallError::Sql { source })?;

    let rows = statement
        .query_map(
            params![query.trim(), repo.orchestrator_id(), top_k as i64],
            |row| {
                let (memory, provenance) = map_memory_row(row)?;
                let score: f64 = row.get(16)?;
                Ok((memory, provenance, score))
            },
        )
        .map_err(|source| MemoryRecallError::Sql { source })?;

    let mut out = Vec::new();
    for (idx, row) in rows.enumerate() {
        let (memory, provenance, bm25_score) =
            row.map_err(|source| MemoryRecallError::Sql { source })?;
        out.push(FullTextCandidate {
            memory,
            provenance,
            rank: idx + 1,
            bm25_score,
        });
    }
    Ok(out)
}

pub fn query_vector(
    repo: &MemoryRepository,
    query_embedding: Option<Vec<f32>>,
    top_k: usize,
) -> Result<VectorQueryOutcome, MemoryRecallError> {
    if top_k == 0 {
        return Ok(VectorQueryOutcome::Ranked(Vec::new()));
    }

    let Some(query_embedding) = query_embedding else {
        return Ok(VectorQueryOutcome::UnavailableMissingQueryEmbedding);
    };

    let query_norm = l2_norm(&query_embedding);
    if query_norm == 0.0 {
        return Ok(VectorQueryOutcome::UnavailableMissingQueryEmbedding);
    }

    let connection = open_connection(repo)?;
    let mut statement = connection
        .prepare(
            "
            SELECT
                m.memory_id,
                m.orchestrator_id,
                m.node_type,
                m.importance,
                m.content,
                m.summary,
                m.confidence,
                m.status,
                m.source_type,
                m.source_path,
                m.conversation_id,
                m.workflow_run_id,
                m.step_id,
                m.captured_by,
                m.created_at,
                m.updated_at,
                e.embedding
            FROM memory_embeddings e
            JOIN memories m
              ON m.orchestrator_id = e.orchestrator_id
             AND m.memory_id = e.memory_id
            WHERE m.orchestrator_id = ?1
              AND m.status != 'retracted'
            ",
        )
        .map_err(|source| MemoryRecallError::Sql { source })?;

    let rows = statement
        .query_map(params![repo.orchestrator_id()], |row| {
            let (memory, provenance) = map_memory_row(row)?;
            let embedding_blob: Option<Vec<u8>> = row.get(16)?;
            Ok((memory, provenance, embedding_blob))
        })
        .map_err(|source| MemoryRecallError::Sql { source })?;

    let mut scored = Vec::new();
    for row in rows {
        let (memory, provenance, embedding_blob) =
            row.map_err(|source| MemoryRecallError::Sql { source })?;
        let Some(blob) = embedding_blob else {
            continue;
        };
        let embedding = decode_embedding(&memory.memory_id, &blob)?;
        if embedding.len() != query_embedding.len() {
            continue;
        }
        let similarity = cosine_similarity(&query_embedding, query_norm, &embedding);
        scored.push((memory, provenance, similarity));
    }

    if scored.is_empty() {
        return Ok(VectorQueryOutcome::UnavailableNoEmbeddings);
    }

    scored.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.memory_id.cmp(&b.0.memory_id))
    });

    let out = scored
        .into_iter()
        .take(top_k)
        .enumerate()
        .map(|(idx, (memory, provenance, similarity))| VectorCandidate {
            memory,
            provenance,
            rank: idx + 1,
            similarity,
        })
        .collect::<Vec<_>>();

    Ok(VectorQueryOutcome::Ranked(out))
}

pub fn hybrid_recall(
    repo: &MemoryRepository,
    request: &HybridRecallRequest,
    options: &MemoryRecallOptions,
    workspace_context: Option<&WorkspaceAccessContext>,
    log_file: &Path,
) -> Result<HybridRecallResult, MemoryRecallError> {
    if request.requesting_orchestrator_id != repo.orchestrator_id() {
        append_memory_log(
            log_file,
            "memory.recall.scope_denied",
            &[
                (
                    "requested_orchestrator_id",
                    Value::String(request.requesting_orchestrator_id.clone()),
                ),
                (
                    "available_orchestrator_id",
                    Value::String(repo.orchestrator_id().to_string()),
                ),
            ],
        )?;
        return Err(MemoryRecallError::CrossOrchestratorDenied {
            requested: request.requesting_orchestrator_id.clone(),
            available: repo.orchestrator_id().to_string(),
        });
    }

    let text_hits = query_full_text(repo, &request.query_text, options.top_k_text)?;
    let vector_out = query_vector(repo, request.query_embedding.clone(), options.top_k_vector)?;
    let (mode, vector_hits) = match vector_out {
        VectorQueryOutcome::Ranked(items) => (HybridRecallResultMode::Hybrid, items),
        VectorQueryOutcome::UnavailableMissingQueryEmbedding
        | VectorQueryOutcome::UnavailableNoEmbeddings => {
            (HybridRecallResultMode::FullTextOnly, Vec::new())
        }
    };

    let candidate_ids = text_hits
        .iter()
        .map(|hit| hit.memory.memory_id.clone())
        .chain(vector_hits.iter().map(|hit| hit.memory.memory_id.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let contradiction_ids = load_unresolved_contradiction_ids(repo, &candidate_ids)?;

    let merged = fuse_rankings(
        &text_hits,
        &vector_hits,
        options.rrf_k,
        options.now_unix_seconds,
        &contradiction_ids,
    );

    let top = merged.into_iter().take(options.top_n).collect::<Vec<_>>();

    if let Some(context) = workspace_context {
        for entry in &top {
            if let Some(path) = &entry.provenance.source_path {
                if enforce_workspace_access(context, std::slice::from_ref(path)).is_err() {
                    append_memory_log(
                        log_file,
                        "memory.recall.source_path_denied",
                        &[("path", Value::String(path.display().to_string()))],
                    )?;
                    return Err(MemoryRecallError::SourcePathAccessDenied {
                        path: path.display().to_string(),
                    });
                }
            }
        }
    }

    let selected_ids = top
        .iter()
        .map(|entry| entry.memory.memory_id.clone())
        .collect::<Vec<_>>();
    let edges = load_edges_for_memory_ids(repo, &selected_ids)?;

    Ok(HybridRecallResult {
        mode,
        memories: top,
        edges,
    })
}

fn fuse_rankings(
    text_hits: &[FullTextCandidate],
    vector_hits: &[VectorCandidate],
    rrf_k: usize,
    now_unix_seconds: i64,
    contradiction_ids: &HashSet<String>,
) -> Vec<HybridRecallMemory> {
    let mut aggregates: HashMap<String, HybridRecallMemory> = HashMap::new();
    let mut rrf_scores: HashMap<String, f64> = HashMap::new();

    for hit in text_hits {
        let key = hit.memory.memory_id.clone();
        let rrf = 1.0 / (rrf_k as f64 + hit.rank as f64);
        *rrf_scores.entry(key.clone()).or_insert(0.0) += rrf;
        aggregates.entry(key).or_insert_with(|| HybridRecallMemory {
            citation: citation_for(&hit.memory, &hit.provenance),
            memory: hit.memory.clone(),
            provenance: hit.provenance.clone(),
            final_score: 0.0,
            unresolved_contradiction: false,
        });
    }

    for hit in vector_hits {
        let key = hit.memory.memory_id.clone();
        let rrf = 1.0 / (rrf_k as f64 + hit.rank as f64);
        *rrf_scores.entry(key.clone()).or_insert(0.0) += rrf;
        aggregates.entry(key).or_insert_with(|| HybridRecallMemory {
            citation: citation_for(&hit.memory, &hit.provenance),
            memory: hit.memory.clone(),
            provenance: hit.provenance.clone(),
            final_score: 0.0,
            unresolved_contradiction: false,
        });
    }

    for (memory_id, entry) in &mut aggregates {
        let mut score = *rrf_scores.get(memory_id).unwrap_or(&0.0);
        score *= 1.0 + (entry.memory.importance as f64 / 100.0);
        score *= entry.memory.confidence as f64;
        score *= recency_weight(entry.memory.updated_at, now_unix_seconds);
        if contradiction_ids.contains(memory_id) {
            score *= 0.80;
            entry.unresolved_contradiction = true;
        }
        entry.final_score = score;
    }

    let mut out = aggregates.into_values().collect::<Vec<_>>();
    out.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.memory.memory_id.cmp(&b.memory.memory_id))
    });
    out
}

fn recency_weight(updated_at: i64, now_unix_seconds: i64) -> f64 {
    let age_seconds = now_unix_seconds.saturating_sub(updated_at).max(0) as f64;
    let days = age_seconds / 86_400.0;
    1.0 / (1.0 + (days / 30.0))
}

fn citation_for(memory: &MemoryNode, provenance: &MemoryProvenanceHandle) -> MemoryCitation {
    MemoryCitation {
        memory_id: memory.memory_id.clone(),
        source_type: source_type_db_label(provenance.source_type).to_string(),
        source_path: provenance.source_path.clone(),
        conversation_id: provenance.conversation_id.clone(),
        workflow_run_id: provenance.workflow_run_id.clone(),
        step_id: provenance.step_id.clone(),
    }
}

fn load_edges_for_memory_ids(
    repo: &MemoryRepository,
    memory_ids: &[String],
) -> Result<Vec<MemoryEdge>, MemoryRecallError> {
    if memory_ids.is_empty() {
        return Ok(Vec::new());
    }

    let wanted = memory_ids.iter().cloned().collect::<BTreeSet<_>>();
    let connection = open_connection(repo)?;
    let mut statement = connection
        .prepare(
            "
            SELECT edge_id, from_memory_id, to_memory_id, edge_type, weight, created_at, reason
            FROM memory_edges
            WHERE orchestrator_id = ?1
            ",
        )
        .map_err(|source| MemoryRecallError::Sql { source })?;

    let rows = statement
        .query_map(params![repo.orchestrator_id()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f32>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|source| MemoryRecallError::Sql { source })?;

    let mut out = Vec::new();
    for row in rows {
        let (edge_id, from_memory_id, to_memory_id, edge_type_raw, weight, created_at, reason) =
            row.map_err(|source| MemoryRecallError::Sql { source })?;
        if !(wanted.contains(&from_memory_id) || wanted.contains(&to_memory_id)) {
            continue;
        }
        out.push(MemoryEdge {
            edge_id,
            from_memory_id,
            to_memory_id,
            edge_type: parse_edge_type(&edge_type_raw)?,
            weight,
            created_at,
            reason,
        });
        if let Some(last) = out.last() {
            validate_edge_weight(last.weight).map_err(|_| {
                MemoryRecallError::InvalidEdgeWeight {
                    edge_id: last.edge_id.clone(),
                    value: last.weight,
                }
            })?;
        }
    }

    Ok(out)
}

fn load_unresolved_contradiction_ids(
    repo: &MemoryRepository,
    candidate_ids: &[String],
) -> Result<HashSet<String>, MemoryRecallError> {
    if candidate_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let wanted = candidate_ids.iter().cloned().collect::<HashSet<_>>();

    let connection = open_connection(repo)?;
    let mut statement = connection
        .prepare(
            "
            SELECT e.from_memory_id, e.to_memory_id, a.status, b.status
            FROM memory_edges e
            JOIN memories a
              ON a.orchestrator_id = e.orchestrator_id
             AND a.memory_id = e.from_memory_id
            JOIN memories b
              ON b.orchestrator_id = e.orchestrator_id
             AND b.memory_id = e.to_memory_id
            WHERE e.orchestrator_id = ?1
              AND e.edge_type = 'Contradicts'
            ",
        )
        .map_err(|source| MemoryRecallError::Sql { source })?;

    let rows = statement
        .query_map(params![repo.orchestrator_id()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|source| MemoryRecallError::Sql { source })?;

    let mut out = HashSet::new();
    for row in rows {
        let (from, to, from_status, to_status) =
            row.map_err(|source| MemoryRecallError::Sql { source })?;
        if from_status == "active" && to_status == "active" {
            if wanted.contains(&from) {
                out.insert(from);
            }
            if wanted.contains(&to) {
                out.insert(to);
            }
        }
    }
    Ok(out)
}

fn open_connection(repo: &MemoryRepository) -> Result<Connection, MemoryRecallError> {
    let connection = Connection::open(repo.database_path()).map_err(|source| {
        MemoryRecallError::Repository(MemoryRepositoryError::Open {
            path: repo.database_path().display().to_string(),
            source,
        })
    })?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|source| MemoryRecallError::Sql { source })?;
    Ok(connection)
}

fn map_memory_row(
    row: &rusqlite::Row<'_>,
) -> Result<(MemoryNode, MemoryProvenanceHandle), rusqlite::Error> {
    let memory_id: String = row.get(0)?;
    let orchestrator_id: String = row.get(1)?;
    let node_type_raw: String = row.get(2)?;
    let importance: i64 = row.get(3)?;
    let content: String = row.get(4)?;
    let summary: String = row.get(5)?;
    let confidence: f32 = row.get(6)?;
    let status_raw: String = row.get(7)?;
    let source_type_raw: String = row.get(8)?;
    let source_path_raw: Option<String> = row.get(9)?;
    let conversation_id: Option<String> = row.get(10)?;
    let workflow_run_id: Option<String> = row.get(11)?;
    let step_id: Option<String> = row.get(12)?;
    let captured_by_raw: String = row.get(13)?;
    let created_at: i64 = row.get(14)?;
    let updated_at: i64 = row.get(15)?;

    let node_type = parse_node_type(&node_type_raw).map_err(to_from_sql_err)?;
    let status = parse_status(&status_raw).map_err(to_from_sql_err)?;
    let source_type = parse_source_type(&source_type_raw).map_err(to_from_sql_err)?;
    let captured_by = parse_captured_by(&captured_by_raw).map_err(to_from_sql_err)?;
    let source_path = source_path_raw.map(PathBuf::from);

    let source = MemorySource {
        source_type,
        source_path: source_path.clone(),
        conversation_id: conversation_id.clone(),
        workflow_run_id: workflow_run_id.clone(),
        step_id: step_id.clone(),
        captured_by,
    };

    if !(0..=100).contains(&importance) {
        return Err(to_from_sql_err(MemoryRecallError::InvalidImportance {
            memory_id: memory_id.clone(),
            value: importance,
        }));
    }
    validate_confidence(confidence).map_err(|_| {
        to_from_sql_err(MemoryRecallError::InvalidConfidence {
            memory_id: memory_id.clone(),
            value: confidence,
        })
    })?;

    let memory = MemoryNode {
        memory_id,
        orchestrator_id,
        node_type,
        importance: importance as u8,
        content,
        summary,
        confidence,
        source,
        status,
        created_at,
        updated_at,
    };

    let provenance = MemoryProvenanceHandle {
        source_type,
        source_path,
        conversation_id,
        workflow_run_id,
        step_id,
        captured_by,
    };

    Ok((memory, provenance))
}

fn parse_node_type(value: &str) -> Result<MemoryNodeType, MemoryRecallError> {
    match value {
        "Fact" => Ok(MemoryNodeType::Fact),
        "Preference" => Ok(MemoryNodeType::Preference),
        "Decision" => Ok(MemoryNodeType::Decision),
        "Identity" => Ok(MemoryNodeType::Identity),
        "Event" => Ok(MemoryNodeType::Event),
        "Observation" => Ok(MemoryNodeType::Observation),
        "Goal" => Ok(MemoryNodeType::Goal),
        "Todo" => Ok(MemoryNodeType::Todo),
        other => Err(MemoryRecallError::InvalidNodeType {
            value: other.to_string(),
        }),
    }
}

fn parse_status(value: &str) -> Result<MemoryStatus, MemoryRecallError> {
    match value {
        "active" => Ok(MemoryStatus::Active),
        "superseded" => Ok(MemoryStatus::Superseded),
        "retracted" => Ok(MemoryStatus::Retracted),
        other => Err(MemoryRecallError::InvalidStatus {
            value: other.to_string(),
        }),
    }
}

fn parse_source_type(value: &str) -> Result<MemorySourceType, MemoryRecallError> {
    match value {
        "workflow_output" => Ok(MemorySourceType::WorkflowOutput),
        "channel_transcript" => Ok(MemorySourceType::ChannelTranscript),
        "ingest_file" => Ok(MemorySourceType::IngestFile),
        "diagnostics" => Ok(MemorySourceType::Diagnostics),
        "manual" => Ok(MemorySourceType::Manual),
        other => Err(MemoryRecallError::InvalidSourceType {
            value: other.to_string(),
        }),
    }
}

fn source_type_db_label(value: MemorySourceType) -> &'static str {
    match value {
        MemorySourceType::WorkflowOutput => "workflow_output",
        MemorySourceType::ChannelTranscript => "channel_transcript",
        MemorySourceType::IngestFile => "ingest_file",
        MemorySourceType::Diagnostics => "diagnostics",
        MemorySourceType::Manual => "manual",
    }
}

fn parse_captured_by(value: &str) -> Result<MemoryCapturedBy, MemoryRecallError> {
    match value {
        "extractor" => Ok(MemoryCapturedBy::Extractor),
        "user" => Ok(MemoryCapturedBy::User),
        "system" => Ok(MemoryCapturedBy::System),
        other => Err(MemoryRecallError::InvalidCapturedBy {
            value: other.to_string(),
        }),
    }
}

fn parse_edge_type(value: &str) -> Result<MemoryEdgeType, MemoryRecallError> {
    match value {
        "RelatedTo" => Ok(MemoryEdgeType::RelatedTo),
        "Updates" => Ok(MemoryEdgeType::Updates),
        "Contradicts" => Ok(MemoryEdgeType::Contradicts),
        "CausedBy" => Ok(MemoryEdgeType::CausedBy),
        "PartOf" => Ok(MemoryEdgeType::PartOf),
        other => Err(MemoryRecallError::InvalidEdgeType {
            value: other.to_string(),
        }),
    }
}

fn to_from_sql_err(err: MemoryRecallError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::other(err.to_string())),
    )
}

fn decode_embedding(memory_id: &str, blob: &[u8]) -> Result<Vec<f32>, MemoryRecallError> {
    if let Ok(parsed) = serde_json::from_slice::<Vec<f32>>(blob) {
        return Ok(parsed);
    }

    if blob.len() % 4 != 0 {
        return Err(MemoryRecallError::InvalidEmbedding {
            memory_id: memory_id.to_string(),
        });
    }

    let mut out = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

fn l2_norm(values: &[f32]) -> f32 {
    values.iter().map(|v| v * v).sum::<f32>().sqrt()
}

fn cosine_similarity(a: &[f32], a_norm: f32, b: &[f32]) -> f32 {
    let b_norm = l2_norm(b);
    if b_norm == 0.0 {
        return 0.0;
    }
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    dot / (a_norm * b_norm)
}

pub(crate) fn append_memory_log(
    path: &Path,
    event: &str,
    fields: &[(&str, Value)],
) -> Result<(), MemoryRecallError> {
    append_memory_event(path, event, fields).map_err(|source| MemoryRecallError::LogWrite {
        path: path.display().to_string(),
        source,
    })
}

pub(crate) fn all_required_bulletin_sections() -> BTreeMap<&'static str, Vec<String>> {
    BTreeMap::from([
        ("knowledge_summary", Vec::new()),
        ("active_goals", Vec::new()),
        ("open_todos", Vec::new()),
        ("recent_decisions", Vec::new()),
        ("preference_profile", Vec::new()),
        ("conflicts_and_uncertainties", Vec::new()),
    ])
}
