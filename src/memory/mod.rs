pub mod bulletin;
pub mod config;
pub mod domain;
pub mod embedding;
pub mod extractor;
pub mod idempotency;
pub mod ingest;
mod logging;
pub mod paths;
pub mod repository;
pub mod retrieval;
pub mod writeback;

pub use bulletin::{
    build_memory_bulletin, bulletin_to_section_map, generate_bulletin_for_message,
    required_bulletin_section_names, BulletinSection, BulletinSectionName, MemoryBulletin,
    MemoryBulletinOptions,
};
pub use config::{
    MemoryBulletinMode, MemoryConfig, MemoryIngestConfig, MemoryRetrievalConfig, MemoryScopeConfig,
};
pub use domain::{
    validate_confidence, validate_edge_weight, validate_importance, MemoryCapturedBy, MemoryEdge,
    MemoryEdgeType, MemoryNode, MemoryNodeType, MemorySource, MemorySourceType, MemoryStatus,
};
pub use embedding::embed_query_text;
pub use extractor::{extract_candidates_from_ingest_file, ExtractedMemory, MemoryExtractionError};
pub use idempotency::compute_ingest_idempotency_key;
pub use ingest::{process_ingest_once, MemoryIngestError};
pub use paths::{
    bootstrap_memory_paths, bootstrap_memory_paths_for_runtime_root, MemoryPathError, MemoryPaths,
};
pub use repository::{MemoryRepository, MemoryRepositoryError, MemorySourceRecord, PersistOutcome};
pub use retrieval::{
    hybrid_recall, query_full_text, query_vector, FullTextCandidate, HybridRecallMemory,
    HybridRecallRequest, HybridRecallResult, HybridRecallResultMode, MemoryCitation,
    MemoryProvenanceHandle, MemoryRecallError, MemoryRecallOptions, VectorCandidate,
    VectorQueryOutcome,
};
pub use writeback::{
    persist_diagnostics_findings, persist_transcript_observation, persist_workflow_output_memories,
    DiagnosticsFindingWriteback, WorkflowOutputWriteback,
};
