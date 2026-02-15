#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write file {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to encode yaml for {path}: {source}")]
    Encode {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("invalid yaml in {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("settings validation failed: {0}")]
    Settings(String),
    #[error("orchestrator validation failed: {0}")]
    Orchestrator(String),
    #[error("orchestrator `{orchestrator_id}` is not configured in settings")]
    MissingOrchestrator { orchestrator_id: String },
    #[error("failed to resolve home directory for global config path")]
    HomeDirectoryUnavailable,
}
