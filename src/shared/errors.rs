#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("failed to create runtime path {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to resolve home directory for runtime state root")]
    HomeDirectoryUnavailable,
    #[error("failed to read runtime state {path}: {source}")]
    ReadState {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse runtime state {path}: {source}")]
    ParseState {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write runtime state {path}: {source}")]
    WriteState {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("supervisor is already running with pid {pid}")]
    AlreadyRunning { pid: u32 },
    #[error("no running supervisor instance")]
    NotRunning,
    #[error("failed to read lock file {path}: {source}")]
    ReadLock {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write lock file {path}: {source}")]
    WriteLock {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to spawn supervisor process: {0}")]
    Spawn(String),
    #[error("failed to stop supervisor process {pid}; process is still alive")]
    StopFailedAlive { pid: u32 },
}
