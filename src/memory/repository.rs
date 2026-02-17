use super::domain::{MemoryCapturedBy, MemoryEdge, MemorySourceType};
use crate::memory::MemoryNode;
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySourceRecord {
    pub orchestrator_id: String,
    pub idempotency_key: String,
    pub source_type: MemorySourceType,
    pub source_path: Option<PathBuf>,
    pub conversation_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub step_id: Option<String>,
    pub captured_by: MemoryCapturedBy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistOutcome {
    Inserted,
    DuplicateSource,
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryRepositoryError {
    #[error("sqlite open failed at {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: rusqlite::Error,
    },
    #[error("failed to create memory database parent {path}: {source}")]
    CreateParent {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("sqlite statement failed: {source}")]
    Sql {
        #[source]
        source: rusqlite::Error,
    },
    #[error("orchestrator scope mismatch: expected {expected}, got {actual}")]
    OrchestratorScopeMismatch { expected: String, actual: String },
    #[error("memory node validation failed: {0}")]
    InvalidNode(String),
    #[error("memory edge validation failed: {0}")]
    InvalidEdge(String),
    #[error("invalid memory source type `{value}` in database")]
    InvalidSourceType { value: String },
    #[error("invalid memory captured_by `{value}` in database")]
    InvalidCapturedBy { value: String },
}

pub struct MemoryRepository {
    db_path: PathBuf,
    orchestrator_id: String,
}

impl MemoryRepository {
    pub fn open(db_path: &Path, orchestrator_id: &str) -> Result<Self, MemoryRepositoryError> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|source| MemoryRepositoryError::CreateParent {
                path: parent.display().to_string(),
                source,
            })?;
        }

        let repo = Self {
            db_path: db_path.to_path_buf(),
            orchestrator_id: orchestrator_id.to_string(),
        };

        // Ensure open is valid now to fail fast.
        let _ = repo.connect()?;
        Ok(repo)
    }

    pub fn ensure_schema(&self) -> Result<(), MemoryRepositoryError> {
        let connection = self.connect()?;
        connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS memories (
                    orchestrator_id TEXT NOT NULL,
                    memory_id TEXT NOT NULL,
                    node_type TEXT NOT NULL,
                    importance INTEGER NOT NULL,
                    content TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    confidence REAL NOT NULL,
                    status TEXT NOT NULL,
                    source_type TEXT NOT NULL,
                    source_path TEXT,
                    conversation_id TEXT,
                    workflow_run_id TEXT,
                    step_id TEXT,
                    captured_by TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    idempotency_key TEXT NOT NULL,
                    PRIMARY KEY (orchestrator_id, memory_id)
                );

                CREATE TABLE IF NOT EXISTS memory_edges (
                    orchestrator_id TEXT NOT NULL,
                    edge_id TEXT NOT NULL,
                    from_memory_id TEXT NOT NULL,
                    to_memory_id TEXT NOT NULL,
                    edge_type TEXT NOT NULL,
                    weight REAL NOT NULL,
                    created_at INTEGER NOT NULL,
                    reason TEXT,
                    PRIMARY KEY (orchestrator_id, edge_id),
                    FOREIGN KEY (orchestrator_id, from_memory_id)
                        REFERENCES memories(orchestrator_id, memory_id)
                        ON DELETE CASCADE,
                    FOREIGN KEY (orchestrator_id, to_memory_id)
                        REFERENCES memories(orchestrator_id, memory_id)
                        ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS memory_sources (
                    orchestrator_id TEXT NOT NULL,
                    idempotency_key TEXT NOT NULL,
                    source_type TEXT NOT NULL,
                    source_path TEXT,
                    conversation_id TEXT,
                    workflow_run_id TEXT,
                    step_id TEXT,
                    captured_by TEXT NOT NULL,
                    processed_at INTEGER NOT NULL,
                    PRIMARY KEY (orchestrator_id, idempotency_key)
                );

                CREATE TABLE IF NOT EXISTS memory_embeddings (
                    orchestrator_id TEXT NOT NULL,
                    memory_id TEXT NOT NULL,
                    embedding BLOB,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (orchestrator_id, memory_id)
                );

                CREATE INDEX IF NOT EXISTS idx_memories_orchestrator_updated
                    ON memories(orchestrator_id, updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_memory_edges_orchestrator_from
                    ON memory_edges(orchestrator_id, from_memory_id);
                CREATE INDEX IF NOT EXISTS idx_memory_sources_orchestrator_processed
                    ON memory_sources(orchestrator_id, processed_at DESC);
                ",
            )
            .map_err(|source| MemoryRepositoryError::Sql { source })?;
        ensure_fts_table(&connection)?;

        Ok(())
    }

    pub fn upsert_nodes_and_edges(
        &self,
        source: &MemorySourceRecord,
        nodes: &[MemoryNode],
        edges: &[MemoryEdge],
    ) -> Result<PersistOutcome, MemoryRepositoryError> {
        self.ensure_scope(&source.orchestrator_id)?;
        for node in nodes {
            node.validate()
                .map_err(|err| MemoryRepositoryError::InvalidNode(err.to_string()))?;
            self.ensure_scope(&node.orchestrator_id)?;
        }
        for edge in edges {
            edge.validate()
                .map_err(|err| MemoryRepositoryError::InvalidEdge(err.to_string()))?;
        }

        let mut connection = self.connect()?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|source| MemoryRepositoryError::Sql { source })?;
        let tx = connection
            .transaction()
            .map_err(|source| MemoryRepositoryError::Sql { source })?;

        let inserted = tx
            .execute(
                "
                INSERT INTO memory_sources (
                    orchestrator_id, idempotency_key, source_type, source_path,
                    conversation_id, workflow_run_id, step_id, captured_by, processed_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%s','now'))
                ON CONFLICT(orchestrator_id, idempotency_key) DO NOTHING
                ",
                params![
                    source.orchestrator_id,
                    source.idempotency_key,
                    source_type_to_db(source.source_type),
                    source
                        .source_path
                        .as_ref()
                        .map(|value| value.display().to_string()),
                    source.conversation_id,
                    source.workflow_run_id,
                    source.step_id,
                    captured_by_to_db(source.captured_by),
                ],
            )
            .map_err(|source| MemoryRepositoryError::Sql { source })?;

        if inserted == 0 {
            tx.rollback()
                .map_err(|source| MemoryRepositoryError::Sql { source })?;
            return Ok(PersistOutcome::DuplicateSource);
        }

        for node in nodes {
            tx.execute(
                "
                INSERT INTO memories (
                    orchestrator_id, memory_id, node_type, importance,
                    content, summary, confidence, status, source_type,
                    source_path, conversation_id, workflow_run_id, step_id,
                    captured_by, created_at, updated_at, idempotency_key
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                ON CONFLICT(orchestrator_id, memory_id) DO UPDATE SET
                    node_type=excluded.node_type,
                    importance=excluded.importance,
                    content=excluded.content,
                    summary=excluded.summary,
                    confidence=excluded.confidence,
                    status=excluded.status,
                    source_type=excluded.source_type,
                    source_path=excluded.source_path,
                    conversation_id=excluded.conversation_id,
                    workflow_run_id=excluded.workflow_run_id,
                    step_id=excluded.step_id,
                    captured_by=excluded.captured_by,
                    updated_at=excluded.updated_at,
                    idempotency_key=excluded.idempotency_key
                ",
                params![
                    node.orchestrator_id,
                    node.memory_id,
                    format!("{:?}", node.node_type),
                    i64::from(node.importance),
                    node.content,
                    node.summary,
                    node.confidence,
                    status_to_db(node.status),
                    source_type_to_db(node.source.source_type),
                    node.source
                        .source_path
                        .as_ref()
                        .map(|value| value.display().to_string()),
                    node.source.conversation_id,
                    node.source.workflow_run_id,
                    node.source.step_id,
                    captured_by_to_db(node.source.captured_by),
                    node.created_at,
                    node.updated_at,
                    source.idempotency_key,
                ],
            )
            .map_err(|source| MemoryRepositoryError::Sql { source })?;

            tx.execute(
                "
                DELETE FROM memory_fts
                WHERE orchestrator_id = ?1 AND memory_id = ?2
                ",
                params![node.orchestrator_id, node.memory_id],
            )
            .map_err(|source| MemoryRepositoryError::Sql { source })?;

            tx.execute(
                "
                INSERT INTO memory_fts (orchestrator_id, memory_id, content, summary)
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![
                    node.orchestrator_id,
                    node.memory_id,
                    node.content,
                    node.summary
                ],
            )
            .map_err(|source| MemoryRepositoryError::Sql { source })?;
        }

        for edge in edges {
            tx.execute(
                "
                INSERT INTO memory_edges (
                    orchestrator_id, edge_id, from_memory_id, to_memory_id,
                    edge_type, weight, created_at, reason
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(orchestrator_id, edge_id) DO UPDATE SET
                    from_memory_id=excluded.from_memory_id,
                    to_memory_id=excluded.to_memory_id,
                    edge_type=excluded.edge_type,
                    weight=excluded.weight,
                    reason=excluded.reason
                ",
                params![
                    self.orchestrator_id,
                    edge.edge_id,
                    edge.from_memory_id,
                    edge.to_memory_id,
                    format!("{:?}", edge.edge_type),
                    edge.weight,
                    edge.created_at,
                    edge.reason,
                ],
            )
            .map_err(|source| MemoryRepositoryError::Sql { source })?;
        }

        tx.commit()
            .map_err(|source| MemoryRepositoryError::Sql { source })?;
        Ok(PersistOutcome::Inserted)
    }

    pub fn count_memories(&self) -> Result<u64, MemoryRepositoryError> {
        let connection = self.connect()?;
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE orchestrator_id = ?1",
                params![self.orchestrator_id],
                |row| row.get(0),
            )
            .map_err(|source| MemoryRepositoryError::Sql { source })?;
        Ok(count as u64)
    }

    pub fn list_sources(&self) -> Result<Vec<MemorySourceRecord>, MemoryRepositoryError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "
                SELECT orchestrator_id, idempotency_key, source_type, source_path,
                       conversation_id, workflow_run_id, step_id, captured_by
                FROM memory_sources
                WHERE orchestrator_id = ?1
                ORDER BY processed_at ASC
                ",
            )
            .map_err(|source| MemoryRepositoryError::Sql { source })?;

        let rows = statement
            .query_map(params![self.orchestrator_id], |row| {
                let source_type_raw: String = row.get(2)?;
                let captured_by_raw: String = row.get(7)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    source_type_raw,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    captured_by_raw,
                ))
            })
            .map_err(|source| MemoryRepositoryError::Sql { source })?;

        let mut out = Vec::new();
        for row in rows {
            let (
                orchestrator_id,
                idempotency_key,
                source_type_raw,
                source_path,
                conversation_id,
                workflow_run_id,
                step_id,
                captured_by_raw,
            ) = row.map_err(|source| MemoryRepositoryError::Sql { source })?;

            let source_type = source_type_from_db(&source_type_raw)?;
            let captured_by = captured_by_from_db(&captured_by_raw)?;
            out.push(MemorySourceRecord {
                orchestrator_id,
                idempotency_key,
                source_type,
                source_path: source_path.map(PathBuf::from),
                conversation_id,
                workflow_run_id,
                step_id,
                captured_by,
            });
        }

        Ok(out)
    }

    pub fn source_exists(&self, idempotency_key: &str) -> Result<bool, MemoryRepositoryError> {
        let connection = self.connect()?;
        let exists = connection
            .query_row(
                "
                SELECT 1 FROM memory_sources
                WHERE orchestrator_id = ?1 AND idempotency_key = ?2
                LIMIT 1
                ",
                params![self.orchestrator_id, idempotency_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|source| MemoryRepositoryError::Sql { source })?
            .is_some();
        Ok(exists)
    }

    pub fn table_names(&self) -> Result<Vec<String>, MemoryRepositoryError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "
                SELECT name FROM sqlite_master
                WHERE type = 'table'
                ORDER BY name ASC
                ",
            )
            .map_err(|source| MemoryRepositoryError::Sql { source })?;

        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|source| MemoryRepositoryError::Sql { source })?;

        let mut names = Vec::new();
        for row in rows {
            names.push(row.map_err(|source| MemoryRepositoryError::Sql { source })?);
        }
        Ok(names)
    }

    fn ensure_scope(&self, orchestrator_id: &str) -> Result<(), MemoryRepositoryError> {
        if orchestrator_id != self.orchestrator_id {
            return Err(MemoryRepositoryError::OrchestratorScopeMismatch {
                expected: self.orchestrator_id.clone(),
                actual: orchestrator_id.to_string(),
            });
        }
        Ok(())
    }

    fn connect(&self) -> Result<Connection, MemoryRepositoryError> {
        let connection =
            Connection::open(&self.db_path).map_err(|source| MemoryRepositoryError::Open {
                path: self.db_path.display().to_string(),
                source,
            })?;
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|source| MemoryRepositoryError::Sql { source })?;
        Ok(connection)
    }
}

fn source_type_to_db(value: MemorySourceType) -> &'static str {
    match value {
        MemorySourceType::WorkflowOutput => "workflow_output",
        MemorySourceType::ChannelTranscript => "channel_transcript",
        MemorySourceType::IngestFile => "ingest_file",
        MemorySourceType::Diagnostics => "diagnostics",
        MemorySourceType::Manual => "manual",
    }
}

fn source_type_from_db(value: &str) -> Result<MemorySourceType, MemoryRepositoryError> {
    match value {
        "workflow_output" => Ok(MemorySourceType::WorkflowOutput),
        "channel_transcript" => Ok(MemorySourceType::ChannelTranscript),
        "ingest_file" => Ok(MemorySourceType::IngestFile),
        "diagnostics" => Ok(MemorySourceType::Diagnostics),
        "manual" => Ok(MemorySourceType::Manual),
        other => Err(MemoryRepositoryError::InvalidSourceType {
            value: other.to_string(),
        }),
    }
}

fn captured_by_to_db(value: MemoryCapturedBy) -> &'static str {
    match value {
        MemoryCapturedBy::Extractor => "extractor",
        MemoryCapturedBy::User => "user",
        MemoryCapturedBy::System => "system",
    }
}

fn captured_by_from_db(value: &str) -> Result<MemoryCapturedBy, MemoryRepositoryError> {
    match value {
        "extractor" => Ok(MemoryCapturedBy::Extractor),
        "user" => Ok(MemoryCapturedBy::User),
        "system" => Ok(MemoryCapturedBy::System),
        other => Err(MemoryRepositoryError::InvalidCapturedBy {
            value: other.to_string(),
        }),
    }
}

fn status_to_db(value: super::domain::MemoryStatus) -> &'static str {
    match value {
        super::domain::MemoryStatus::Active => "active",
        super::domain::MemoryStatus::Superseded => "superseded",
        super::domain::MemoryStatus::Retracted => "retracted",
    }
}

fn ensure_fts_table(connection: &Connection) -> Result<(), MemoryRepositoryError> {
    let existing_sql: Option<String> = connection
        .query_row(
            "
            SELECT sql
            FROM sqlite_master
            WHERE type = 'table' AND name = 'memory_fts'
            LIMIT 1
            ",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| MemoryRepositoryError::Sql { source })?;

    if let Some(sql) = existing_sql {
        if !sql.to_ascii_uppercase().contains("VIRTUAL TABLE") {
            connection
                .execute_batch("DROP TABLE memory_fts;")
                .map_err(|source| MemoryRepositoryError::Sql { source })?;
        }
    }

    connection
        .execute_batch(
            "
            CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts
            USING fts5(
                orchestrator_id UNINDEXED,
                memory_id UNINDEXED,
                content,
                summary
            );
            ",
        )
        .map_err(|source| MemoryRepositoryError::Sql { source })?;

    Ok(())
}
