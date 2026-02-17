use super::embedding::upsert_embeddings_for_nodes_best_effort;
use super::extractor::{extract_candidates_from_ingest_file, MemoryExtractionError};
use super::idempotency::compute_ingest_idempotency_key;
use super::logging::append_memory_event;
use super::repository::{MemoryRepository, MemorySourceRecord, PersistOutcome};
use super::{MemoryCapturedBy, MemoryPaths, MemorySourceType};
use serde::Serialize;
use serde_json::Value;
use std::ffi::OsStr;
use std::fs::{self};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum MemoryIngestError {
    #[error("failed to read ingest directory {path}: {source}")]
    ReadDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read ingest file {path}: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to canonicalize ingest file {path}: {source}")]
    Canonicalize {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to move file from {from} to {to}: {source}")]
    MoveFile {
        from: String,
        to: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write manifest {path}: {source}")]
    WriteManifest {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("repository error: {0}")]
    Repository(String),
}

pub fn process_ingest_once(
    paths: &MemoryPaths,
    orchestrator_id: &str,
    max_file_size_mb: u64,
) -> Result<(), MemoryIngestError> {
    let repo = MemoryRepository::open(&paths.database, orchestrator_id)
        .map_err(|err| MemoryIngestError::Repository(err.to_string()))?;
    repo.ensure_schema()
        .map_err(|err| MemoryIngestError::Repository(err.to_string()))?;

    let files = discover_ingest_files(&paths.ingest)?;
    for source_path in files {
        process_one_file(
            paths,
            orchestrator_id,
            max_file_size_mb,
            &repo,
            &source_path,
        )?;
    }

    Ok(())
}

fn discover_ingest_files(ingest_root: &Path) -> Result<Vec<PathBuf>, MemoryIngestError> {
    let entries = fs::read_dir(ingest_root).map_err(|source| MemoryIngestError::ReadDir {
        path: ingest_root.display().to_string(),
        source,
    })?;

    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| MemoryIngestError::ReadDir {
            path: ingest_root.display().to_string(),
            source,
        })?;

        let path = entry.path();
        if path.is_file() {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

fn process_one_file(
    paths: &MemoryPaths,
    orchestrator_id: &str,
    max_file_size_mb: u64,
    repo: &MemoryRepository,
    ingest_path: &Path,
) -> Result<(), MemoryIngestError> {
    let bytes = fs::read(ingest_path).map_err(|source| MemoryIngestError::ReadFile {
        path: ingest_path.display().to_string(),
        source,
    })?;

    let ingest_canonical =
        ingest_path
            .canonicalize()
            .map_err(|source| MemoryIngestError::Canonicalize {
                path: ingest_path.display().to_string(),
                source,
            })?;
    let idempotency_key = compute_ingest_idempotency_key(&ingest_canonical, &bytes);

    if bytes.len() > max_bytes(max_file_size_mb) {
        reject_file(
            paths,
            ingest_path,
            Some(&idempotency_key),
            "file_too_large",
            &format!("file size exceeds configured {} MB", max_file_size_mb),
        )?;
        append_ingest_event(
            &paths.log_file,
            "memory.ingest.rejected",
            &[
                ("status", Value::String("rejected".to_string())),
                (
                    "orchestrator_id",
                    Value::String(orchestrator_id.to_string()),
                ),
                (
                    "source_path",
                    Value::String(ingest_path.display().to_string()),
                ),
                ("idempotency_key", Value::String(idempotency_key.clone())),
                ("reason_code", Value::String("file_too_large".to_string())),
            ],
        )?;
        return Ok(());
    }

    let extension = ingest_path
        .extension()
        .and_then(OsStr::to_str)
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(extension.as_str(), "txt" | "md" | "json") {
        reject_file(
            paths,
            ingest_path,
            Some(&idempotency_key),
            "unsupported_file_type",
            &format!("unsupported ingest extension `{}`", extension),
        )?;
        append_ingest_event(
            &paths.log_file,
            "memory.ingest.rejected",
            &[
                ("status", Value::String("rejected".to_string())),
                (
                    "orchestrator_id",
                    Value::String(orchestrator_id.to_string()),
                ),
                (
                    "source_path",
                    Value::String(ingest_path.display().to_string()),
                ),
                ("idempotency_key", Value::String(idempotency_key.clone())),
                (
                    "reason_code",
                    Value::String("unsupported_file_type".to_string()),
                ),
            ],
        )?;
        return Ok(());
    }

    if repo
        .source_exists(&idempotency_key)
        .map_err(|err| MemoryIngestError::Repository(err.to_string()))?
    {
        let processed_path = move_with_collision(ingest_path, &paths.ingest_processed)?;
        write_processed_manifest(
            &processed_path,
            &ProcessedManifest {
                status: "duplicate",
                idempotency_key: idempotency_key.as_str(),
                memories_written: 0,
                edges_written: 0,
            },
        )?;
        append_ingest_event(
            &paths.log_file,
            "memory.ingest.duplicate",
            &[
                ("status", Value::String("duplicate".to_string())),
                (
                    "orchestrator_id",
                    Value::String(orchestrator_id.to_string()),
                ),
                (
                    "source_path",
                    Value::String(processed_path.display().to_string()),
                ),
                ("idempotency_key", Value::String(idempotency_key.clone())),
            ],
        )?;
        return Ok(());
    }

    let processed_path = move_with_collision(ingest_path, &paths.ingest_processed)?;
    let processed_canonical =
        processed_path
            .canonicalize()
            .map_err(|source| MemoryIngestError::Canonicalize {
                path: processed_path.display().to_string(),
                source,
            })?;

    let extracted =
        match extract_candidates_from_ingest_file(orchestrator_id, &processed_canonical, &bytes) {
            Ok(value) => value,
            Err(err) => {
                move_to_rejected_extraction(paths, &processed_path, Some(&idempotency_key), &err)?;
                append_ingest_event(
                    &paths.log_file,
                    "memory.ingest.rejected",
                    &[
                        ("status", Value::String("rejected".to_string())),
                        (
                            "orchestrator_id",
                            Value::String(orchestrator_id.to_string()),
                        ),
                        (
                            "source_path",
                            Value::String(processed_path.display().to_string()),
                        ),
                        ("idempotency_key", Value::String(idempotency_key.clone())),
                        (
                            "reason_code",
                            Value::String(rejection_code(&err).to_string()),
                        ),
                    ],
                )?;
                return Ok(());
            }
        };

    let source = MemorySourceRecord {
        orchestrator_id: orchestrator_id.to_string(),
        idempotency_key: idempotency_key.clone(),
        source_type: MemorySourceType::IngestFile,
        source_path: Some(processed_canonical),
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::Extractor,
    };

    match repo.upsert_nodes_and_edges(&source, &extracted.nodes, &extracted.edges) {
        Ok(PersistOutcome::Inserted) => {
            upsert_embeddings_for_nodes_best_effort(repo, &extracted.nodes, timestamp_now());
            write_processed_manifest(
                &processed_path,
                &ProcessedManifest {
                    status: "processed",
                    idempotency_key: idempotency_key.as_str(),
                    memories_written: extracted.nodes.len(),
                    edges_written: extracted.edges.len(),
                },
            )?;
            append_ingest_event(
                &paths.log_file,
                "memory.ingest.processed",
                &[
                    ("status", Value::String("processed".to_string())),
                    (
                        "orchestrator_id",
                        Value::String(orchestrator_id.to_string()),
                    ),
                    (
                        "source_path",
                        Value::String(processed_path.display().to_string()),
                    ),
                    ("idempotency_key", Value::String(idempotency_key.clone())),
                    ("memories_written", Value::from(extracted.nodes.len())),
                    ("edges_written", Value::from(extracted.edges.len())),
                ],
            )?;
            Ok(())
        }
        Ok(PersistOutcome::DuplicateSource) => {
            write_processed_manifest(
                &processed_path,
                &ProcessedManifest {
                    status: "duplicate",
                    idempotency_key: idempotency_key.as_str(),
                    memories_written: 0,
                    edges_written: 0,
                },
            )?;
            append_ingest_event(
                &paths.log_file,
                "memory.ingest.duplicate",
                &[
                    ("status", Value::String("duplicate".to_string())),
                    (
                        "orchestrator_id",
                        Value::String(orchestrator_id.to_string()),
                    ),
                    (
                        "source_path",
                        Value::String(processed_path.display().to_string()),
                    ),
                    ("idempotency_key", Value::String(idempotency_key.clone())),
                ],
            )?;
            Ok(())
        }
        Err(err) => {
            move_to_rejected_rejection(
                paths,
                &processed_path,
                Some(&idempotency_key),
                "repository_error",
                &err.to_string(),
            )?;
            append_ingest_event(
                &paths.log_file,
                "memory.ingest.rejected",
                &[
                    ("status", Value::String("rejected".to_string())),
                    (
                        "orchestrator_id",
                        Value::String(orchestrator_id.to_string()),
                    ),
                    (
                        "source_path",
                        Value::String(processed_path.display().to_string()),
                    ),
                    ("idempotency_key", Value::String(idempotency_key.clone())),
                    ("reason_code", Value::String("repository_error".to_string())),
                ],
            )?;
            Ok(())
        }
    }
}

fn move_to_rejected(
    paths: &MemoryPaths,
    processed_or_ingest_path: &Path,
    idempotency_key: Option<&str>,
    code: &'static str,
    message: &str,
) -> Result<(), MemoryIngestError> {
    let rejected_path = move_with_collision(processed_or_ingest_path, &paths.ingest_rejected)?;
    write_rejection_manifest(
        &rejected_path,
        &RejectionManifest {
            status: "rejected",
            source_path: rejected_path.display().to_string(),
            idempotency_key,
            error: RejectionError {
                code,
                message: message.to_string(),
            },
        },
    )?;
    Ok(())
}

fn move_to_rejected_extraction(
    paths: &MemoryPaths,
    processed_or_ingest_path: &Path,
    idempotency_key: Option<&str>,
    error: &MemoryExtractionError,
) -> Result<(), MemoryIngestError> {
    move_to_rejected(
        paths,
        processed_or_ingest_path,
        idempotency_key,
        rejection_code(error),
        &error.to_string(),
    )
}

fn move_to_rejected_rejection(
    paths: &MemoryPaths,
    processed_or_ingest_path: &Path,
    idempotency_key: Option<&str>,
    code: &'static str,
    message: &str,
) -> Result<(), MemoryIngestError> {
    move_to_rejected(
        paths,
        processed_or_ingest_path,
        idempotency_key,
        code,
        message,
    )
}

fn reject_file(
    paths: &MemoryPaths,
    ingest_path: &Path,
    idempotency_key: Option<&str>,
    code: &'static str,
    message: &str,
) -> Result<(), MemoryIngestError> {
    let rejected_path = move_with_collision(ingest_path, &paths.ingest_rejected)?;
    write_rejection_manifest(
        &rejected_path,
        &RejectionManifest {
            status: "rejected",
            source_path: rejected_path.display().to_string(),
            idempotency_key,
            error: RejectionError {
                code,
                message: message.to_string(),
            },
        },
    )
}

fn move_with_collision(from: &Path, dest_dir: &Path) -> Result<PathBuf, MemoryIngestError> {
    fs::create_dir_all(dest_dir).map_err(|source| MemoryIngestError::MoveFile {
        from: from.display().to_string(),
        to: dest_dir.display().to_string(),
        source,
    })?;

    let file_name = from
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("ingest-file");
    let mut candidate = dest_dir.join(file_name);
    let mut suffix = 1usize;

    while candidate.exists() {
        let next = format!("{}.{}", file_name, suffix);
        candidate = dest_dir.join(next);
        suffix = suffix.saturating_add(1);
    }

    fs::rename(from, &candidate).map_err(|source| MemoryIngestError::MoveFile {
        from: from.display().to_string(),
        to: candidate.display().to_string(),
        source,
    })?;

    Ok(candidate)
}

fn write_processed_manifest(
    source_file: &Path,
    manifest: &ProcessedManifest<'_>,
) -> Result<(), MemoryIngestError> {
    let manifest_path = manifest_path(source_file, "processed");
    let encoded = serde_json::to_vec_pretty(manifest)
        .expect("processed ingest manifest serialization should not fail");
    fs::write(&manifest_path, encoded).map_err(|source| MemoryIngestError::WriteManifest {
        path: manifest_path.display().to_string(),
        source,
    })
}

fn write_rejection_manifest(
    source_file: &Path,
    manifest: &RejectionManifest<'_>,
) -> Result<(), MemoryIngestError> {
    let manifest_path = manifest_path(source_file, "rejection");
    let encoded = serde_json::to_vec_pretty(manifest)
        .expect("rejected ingest manifest serialization should not fail");
    fs::write(&manifest_path, encoded).map_err(|source| MemoryIngestError::WriteManifest {
        path: manifest_path.display().to_string(),
        source,
    })
}

fn manifest_path(source_file: &Path, kind: &str) -> PathBuf {
    let file_name = source_file
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("artifact");
    source_file.with_file_name(format!("{file_name}.{kind}.json"))
}

fn rejection_code(error: &MemoryExtractionError) -> &'static str {
    match error {
        MemoryExtractionError::UnsupportedFileType { .. } => "unsupported_file_type",
        MemoryExtractionError::InvalidJson(_) => "invalid_json",
        MemoryExtractionError::InvalidSourcePath => "invalid_source_path",
        MemoryExtractionError::JsonScopeMismatch { .. } => "scope_mismatch",
        MemoryExtractionError::EmptyContent { .. } => "empty_content",
        MemoryExtractionError::EmptySummary { .. } => "empty_summary",
    }
}

fn max_bytes(max_file_size_mb: u64) -> usize {
    let one_mb = 1024u64 * 1024u64;
    let bytes = max_file_size_mb.saturating_mul(one_mb);
    usize::try_from(bytes).unwrap_or(usize::MAX)
}

fn timestamp_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn append_ingest_event(
    path: &Path,
    event: &str,
    fields: &[(&str, Value)],
) -> Result<(), MemoryIngestError> {
    append_memory_event(path, event, fields).map_err(|source| MemoryIngestError::WriteManifest {
        path: path.display().to_string(),
        source,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessedManifest<'a> {
    status: &'a str,
    idempotency_key: &'a str,
    memories_written: usize,
    edges_written: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RejectionManifest<'a> {
    status: &'a str,
    source_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotency_key: Option<&'a str>,
    error: RejectionError<'a>,
}

#[derive(Debug, Serialize)]
struct RejectionError<'a> {
    code: &'a str,
    message: String,
}
