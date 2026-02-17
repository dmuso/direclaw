use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum MemoryPathError {
    #[error("failed to canonicalize memory runtime root {path}: {source}")]
    Canonicalize {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create memory path {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub ingest: PathBuf,
    pub ingest_processed: PathBuf,
    pub ingest_rejected: PathBuf,
    pub bulletins: PathBuf,
    pub logs_dir: PathBuf,
    pub log_file: PathBuf,
}

impl MemoryPaths {
    pub fn from_runtime_root(runtime_root: &Path) -> Self {
        let root = runtime_root.join("memory");
        Self {
            database: root.join("memory.db"),
            ingest: root.join("ingest"),
            ingest_processed: root.join("ingest/processed"),
            ingest_rejected: root.join("ingest/rejected"),
            bulletins: root.join("bulletins"),
            logs_dir: root.join("logs"),
            log_file: root.join("logs/memory.log"),
            root,
        }
    }

    pub fn from_runtime_root_canonical(runtime_root: &Path) -> Result<Self, MemoryPathError> {
        let canonical =
            fs::canonicalize(runtime_root).map_err(|source| MemoryPathError::Canonicalize {
                path: runtime_root.display().to_string(),
                source,
            })?;
        Ok(Self::from_runtime_root(&canonical))
    }

    pub fn required_directories(&self) -> Vec<PathBuf> {
        vec![
            self.root.clone(),
            self.ingest.clone(),
            self.ingest_processed.clone(),
            self.ingest_rejected.clone(),
            self.bulletins.clone(),
            self.logs_dir.clone(),
        ]
    }
}

pub fn bootstrap_memory_paths(paths: &MemoryPaths) -> Result<(), MemoryPathError> {
    for path in paths.required_directories() {
        fs::create_dir_all(&path).map_err(|source| MemoryPathError::CreateDir {
            path: path.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

pub fn bootstrap_memory_paths_for_runtime_root(runtime_root: &Path) -> Result<(), MemoryPathError> {
    let paths = MemoryPaths::from_runtime_root(runtime_root);
    bootstrap_memory_paths(&paths)
}
