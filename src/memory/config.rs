use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryBulletinMode {
    EveryMessage,
}

fn default_enabled() -> bool {
    true
}

fn default_bulletin_mode() -> MemoryBulletinMode {
    MemoryBulletinMode::EveryMessage
}

fn default_retrieval_top_n() -> usize {
    20
}

fn default_retrieval_rrf_k() -> usize {
    60
}

fn default_ingest_enabled() -> bool {
    true
}

fn default_ingest_max_file_size_mb() -> u64 {
    25
}

fn default_worker_interval_seconds() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_bulletin_mode")]
    pub bulletin_mode: MemoryBulletinMode,
    #[serde(default)]
    pub retrieval: MemoryRetrievalConfig,
    #[serde(default)]
    pub ingest: MemoryIngestConfig,
    #[serde(default = "default_worker_interval_seconds")]
    pub worker_interval_seconds: u64,
    #[serde(default)]
    pub scope: MemoryScopeConfig,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            bulletin_mode: default_bulletin_mode(),
            retrieval: MemoryRetrievalConfig::default(),
            ingest: MemoryIngestConfig::default(),
            worker_interval_seconds: default_worker_interval_seconds(),
            scope: MemoryScopeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryRetrievalConfig {
    #[serde(default = "default_retrieval_top_n")]
    pub top_n: usize,
    #[serde(default = "default_retrieval_rrf_k")]
    pub rrf_k: usize,
}

impl Default for MemoryRetrievalConfig {
    fn default() -> Self {
        Self {
            top_n: default_retrieval_top_n(),
            rrf_k: default_retrieval_rrf_k(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryIngestConfig {
    #[serde(default = "default_ingest_enabled")]
    pub enabled: bool,
    #[serde(default = "default_ingest_max_file_size_mb")]
    pub max_file_size_mb: u64,
}

impl Default for MemoryIngestConfig {
    fn default() -> Self {
        Self {
            enabled: default_ingest_enabled(),
            max_file_size_mb: default_ingest_max_file_size_mb(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryScopeConfig {
    pub cross_orchestrator: bool,
}

impl MemoryConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.retrieval.top_n == 0 {
            return Err("memory.retrieval.top_n must be >= 1".to_string());
        }
        if self.retrieval.rrf_k == 0 {
            return Err("memory.retrieval.rrf_k must be >= 1".to_string());
        }
        if self.ingest.max_file_size_mb == 0 {
            return Err("memory.ingest.max_file_size_mb must be >= 1".to_string());
        }
        if self.worker_interval_seconds == 0 {
            return Err("memory.worker_interval_seconds must be >= 1".to_string());
        }
        if self.scope.cross_orchestrator {
            return Err(
                "memory.scope.cross_orchestrator=true is not supported in v1; expected false"
                    .to_string(),
            );
        }
        Ok(())
    }
}
