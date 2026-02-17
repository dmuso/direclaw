use crate::orchestration::error::OrchestratorError;
use crate::orchestration::selector::{SelectorRequest, SelectorResult};
use crate::queue::IncomingMessage;
use crate::shared::logging::{append_orchestrator_log_line, orchestrator_log_path};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

pub struct SelectorArtifactStore {
    state_root: PathBuf,
}

impl SelectorArtifactStore {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
        }
    }

    pub fn persist_message_snapshot(
        &self,
        inbound: &IncomingMessage,
    ) -> Result<PathBuf, OrchestratorError> {
        let path = self
            .state_root
            .join("orchestrator/artifacts")
            .join(format!("message-{}.json", inbound.message_id));
        self.write_json(&path, inbound)
    }

    pub fn persist_selector_request(
        &self,
        request: &SelectorRequest,
    ) -> Result<PathBuf, OrchestratorError> {
        let path = self
            .state_root
            .join("orchestrator/artifacts")
            .join(format!("selector-request-{}.json", request.selector_id));
        self.write_json(&path, request)
    }

    pub fn move_request_to_processing(
        &self,
        selector_id: &str,
    ) -> Result<PathBuf, OrchestratorError> {
        let incoming = self
            .state_root
            .join("orchestrator/artifacts")
            .join(format!("selector-request-{selector_id}.json"));
        let processing = self
            .state_root
            .join("orchestrator/artifacts")
            .join(format!("selector-processing-{selector_id}.json"));
        if let Some(parent) = processing.parent() {
            fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
        }
        fs::rename(&incoming, &processing).map_err(|source| io_error(&incoming, source))?;
        Ok(processing)
    }

    pub fn persist_selector_result(
        &self,
        result: &SelectorResult,
    ) -> Result<PathBuf, OrchestratorError> {
        let path = self
            .state_root
            .join("orchestrator/artifacts")
            .join(format!("selector-result-{}.json", result.selector_id));
        self.write_json(&path, result)
    }

    pub fn persist_selector_log(
        &self,
        selector_id: &str,
        content: &str,
    ) -> Result<PathBuf, OrchestratorError> {
        let line = format!("selector_id={selector_id} {content}");
        append_orchestrator_log_line(&self.state_root, &line).map_err(|source| {
            io_error(orchestrator_log_path(&self.state_root).as_path(), source)
        })?;
        let path = orchestrator_log_path(&self.state_root);
        Ok(path)
    }

    fn write_json<T: Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<PathBuf, OrchestratorError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
        }
        let body = serde_json::to_vec_pretty(value).map_err(|source| json_error(path, source))?;
        fs::write(path, body).map_err(|source| io_error(path, source))?;
        Ok(path.to_path_buf())
    }
}

fn io_error(path: &Path, source: std::io::Error) -> OrchestratorError {
    OrchestratorError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn json_error(path: &Path, source: serde_json::Error) -> OrchestratorError {
    OrchestratorError::Json {
        path: path.display().to_string(),
        source,
    }
}
