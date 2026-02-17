pub mod config;
pub mod domain;
pub mod extractor;
pub mod idempotency;
pub mod ingest;
pub mod paths;
pub mod repository;

pub use config::{
    MemoryBulletinMode, MemoryConfig, MemoryIngestConfig, MemoryRetrievalConfig, MemoryScopeConfig,
};
pub use domain::{
    validate_confidence, validate_edge_weight, validate_importance, MemoryCapturedBy, MemoryEdge,
    MemoryEdgeType, MemoryNode, MemoryNodeType, MemorySource, MemorySourceType, MemoryStatus,
};
pub use extractor::{extract_candidates_from_ingest_file, ExtractedMemory, MemoryExtractionError};
pub use idempotency::compute_ingest_idempotency_key;
pub use ingest::{process_ingest_once, MemoryIngestError};
pub use paths::{
    bootstrap_memory_paths, bootstrap_memory_paths_for_runtime_root, MemoryPathError, MemoryPaths,
};
pub use repository::{MemoryRepository, MemoryRepositoryError, MemorySourceRecord, PersistOutcome};
