use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum MemoryDomainError {
    #[error("importance must be in range 0..=100; got {value}")]
    ImportanceOutOfRange { value: u8 },
    #[error("confidence must be in range 0.0..=1.0; got {value}")]
    ConfidenceOutOfRange { value: f32 },
    #[error("edge weight must be in range 0.0..=1.0; got {value}")]
    EdgeWeightOutOfRange { value: f32 },
    #[error("required field `{field}` must be non-empty")]
    MissingField { field: &'static str },
    #[error("source_path must be absolute when provided")]
    SourcePathMustBeAbsolute,
    #[error("source_path must be canonical when source_type is ingest_file")]
    SourcePathMustBeCanonical,
    #[error("source_path must resolve to an existing file when source_type is ingest_file")]
    SourcePathCanonicalizeFailed,
    #[error("source type `workflow_output` requires workflow_run_id")]
    MissingWorkflowRunId,
    #[error("source type `workflow_output` requires step_id")]
    MissingStepId,
    #[error("source type `channel_transcript` requires conversation_id")]
    MissingConversationId,
    #[error("source type `ingest_file` requires source_path")]
    MissingSourcePath,
}

pub fn validate_importance(value: u8) -> Result<(), MemoryDomainError> {
    if value > 100 {
        return Err(MemoryDomainError::ImportanceOutOfRange { value });
    }
    Ok(())
}

pub fn validate_confidence(value: f32) -> Result<(), MemoryDomainError> {
    if !(0.0..=1.0).contains(&value) {
        return Err(MemoryDomainError::ConfidenceOutOfRange { value });
    }
    Ok(())
}

pub fn validate_edge_weight(value: f32) -> Result<(), MemoryDomainError> {
    if !(0.0..=1.0).contains(&value) {
        return Err(MemoryDomainError::EdgeWeightOutOfRange { value });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum MemoryNodeType {
    Fact,
    Preference,
    Decision,
    Identity,
    Event,
    Observation,
    Goal,
    Todo,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum MemoryEdgeType {
    RelatedTo,
    Updates,
    Contradicts,
    CausedBy,
    PartOf,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Superseded,
    Retracted,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemorySourceType {
    WorkflowOutput,
    ChannelTranscript,
    IngestFile,
    Diagnostics,
    Manual,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCapturedBy {
    Extractor,
    User,
    System,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemorySource {
    pub source_type: MemorySourceType,
    #[serde(default)]
    pub source_path: Option<PathBuf>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub workflow_run_id: Option<String>,
    #[serde(default)]
    pub step_id: Option<String>,
    pub captured_by: MemoryCapturedBy,
}

impl MemorySource {
    pub fn validate(&self) -> Result<(), MemoryDomainError> {
        if let Some(path) = &self.source_path {
            if !path.is_absolute() {
                return Err(MemoryDomainError::SourcePathMustBeAbsolute);
            }
        }

        match self.source_type {
            MemorySourceType::WorkflowOutput => {
                if self
                    .workflow_run_id
                    .as_ref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(MemoryDomainError::MissingWorkflowRunId);
                }
                if self
                    .step_id
                    .as_ref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(MemoryDomainError::MissingStepId);
                }
            }
            MemorySourceType::ChannelTranscript => {
                if self
                    .conversation_id
                    .as_ref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(MemoryDomainError::MissingConversationId);
                }
            }
            MemorySourceType::IngestFile => {
                let path = self
                    .source_path
                    .as_ref()
                    .ok_or(MemoryDomainError::MissingSourcePath)?;
                let canonical = fs::canonicalize(path)
                    .map_err(|_| MemoryDomainError::SourcePathCanonicalizeFailed)?;
                if canonical != *path {
                    return Err(MemoryDomainError::SourcePathMustBeCanonical);
                }
            }
            MemorySourceType::Diagnostics | MemorySourceType::Manual => {}
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryNode {
    pub memory_id: String,
    pub orchestrator_id: String,
    #[serde(rename = "type")]
    pub node_type: MemoryNodeType,
    pub importance: u8,
    pub content: String,
    pub summary: String,
    pub confidence: f32,
    pub source: MemorySource,
    pub status: MemoryStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

impl MemoryNode {
    pub fn validate(&self) -> Result<(), MemoryDomainError> {
        if self.memory_id.trim().is_empty() {
            return Err(MemoryDomainError::MissingField { field: "memory_id" });
        }
        if self.orchestrator_id.trim().is_empty() {
            return Err(MemoryDomainError::MissingField {
                field: "orchestrator_id",
            });
        }
        if self.content.trim().is_empty() {
            return Err(MemoryDomainError::MissingField { field: "content" });
        }
        if self.summary.trim().is_empty() {
            return Err(MemoryDomainError::MissingField { field: "summary" });
        }
        validate_importance(self.importance)?;
        validate_confidence(self.confidence)?;
        self.source.validate()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEdge {
    pub edge_id: String,
    pub from_memory_id: String,
    pub to_memory_id: String,
    pub edge_type: MemoryEdgeType,
    pub weight: f32,
    pub created_at: i64,
    #[serde(default)]
    pub reason: Option<String>,
}

impl MemoryEdge {
    pub fn validate(&self) -> Result<(), MemoryDomainError> {
        if self.edge_id.trim().is_empty() {
            return Err(MemoryDomainError::MissingField { field: "edge_id" });
        }
        if self.from_memory_id.trim().is_empty() {
            return Err(MemoryDomainError::MissingField {
                field: "from_memory_id",
            });
        }
        if self.to_memory_id.trim().is_empty() {
            return Err(MemoryDomainError::MissingField {
                field: "to_memory_id",
            });
        }
        validate_edge_weight(self.weight)
    }
}
