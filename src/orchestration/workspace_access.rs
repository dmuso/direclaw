use crate::config::{AgentConfig, OrchestratorConfig, Settings};
use crate::orchestrator::OrchestratorError;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceAccessContext {
    pub orchestrator_id: String,
    pub private_workspace_root: PathBuf,
    pub shared_workspace_roots: BTreeMap<String, PathBuf>,
}

pub fn resolve_workspace_access_context(
    settings: &Settings,
    orchestrator_id: &str,
) -> Result<WorkspaceAccessContext, OrchestratorError> {
    let private_workspace = canonicalize_absolute_path_if_exists(
        &settings.resolve_private_workspace(orchestrator_id)?,
    )?;
    let orchestrator = settings.orchestrators.get(orchestrator_id).ok_or_else(|| {
        OrchestratorError::Config(format!(
            "missing orchestrator `{orchestrator_id}` in settings"
        ))
    })?;

    let mut shared_workspace_roots = BTreeMap::new();
    for grant in &orchestrator.shared_access {
        let shared = settings.shared_workspaces.get(grant).ok_or_else(|| {
            OrchestratorError::Config(format!(
                "orchestrator `{orchestrator_id}` references unknown shared workspace `{grant}`"
            ))
        })?;
        let normalized = canonicalize_absolute_path_if_exists(shared)?;
        shared_workspace_roots.insert(grant.clone(), normalized);
    }

    Ok(WorkspaceAccessContext {
        orchestrator_id: orchestrator_id.to_string(),
        private_workspace_root: private_workspace,
        shared_workspace_roots,
    })
}

pub fn verify_orchestrator_workspace_access(
    settings: &Settings,
    orchestrator_id: &str,
    orchestrator: &OrchestratorConfig,
) -> Result<WorkspaceAccessContext, OrchestratorError> {
    let workspace_context = resolve_workspace_access_context(settings, orchestrator_id)?;
    let requested_paths =
        collect_orchestrator_requested_paths(&workspace_context, settings, orchestrator)?;
    enforce_workspace_access(&workspace_context, &requested_paths)?;
    Ok(workspace_context)
}

pub fn enforce_workspace_access(
    context: &WorkspaceAccessContext,
    requested_paths: &[PathBuf],
) -> Result<(), OrchestratorError> {
    for requested in requested_paths {
        let normalized = canonicalize_absolute_path_if_exists(requested)?;
        if normalized.starts_with(&context.private_workspace_root) {
            continue;
        }
        if context
            .shared_workspace_roots
            .values()
            .any(|root| normalized.starts_with(root))
        {
            continue;
        }
        return Err(OrchestratorError::WorkspaceAccessDenied {
            orchestrator_id: context.orchestrator_id.clone(),
            path: normalized.display().to_string(),
        });
    }
    Ok(())
}

pub fn resolve_agent_workspace_root(
    private_workspace_root: &Path,
    agent_id: &str,
    agent: &AgentConfig,
) -> PathBuf {
    match &agent.private_workspace {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => private_workspace_root.join(path),
        None => private_workspace_root.join("agents").join(agent_id),
    }
}

fn canonicalize_absolute_path_if_exists(path: &Path) -> Result<PathBuf, OrchestratorError> {
    match fs::canonicalize(path) {
        Ok(canonical) => normalize_absolute_path(&canonical),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => normalize_absolute_path(path),
        Err(err) => Err(io_error(path, err)),
    }
}

fn collect_orchestrator_requested_paths(
    context: &WorkspaceAccessContext,
    settings: &Settings,
    orchestrator: &OrchestratorConfig,
) -> Result<Vec<PathBuf>, OrchestratorError> {
    let mut requested = vec![context.private_workspace_root.clone()];
    for (agent_id, agent) in &orchestrator.agents {
        requested.push(resolve_agent_workspace_root(
            &context.private_workspace_root,
            agent_id,
            agent,
        ));
        for shared in &agent.shared_access {
            let path = settings.shared_workspaces.get(shared).ok_or_else(|| {
                OrchestratorError::Config(format!(
                    "agent `{agent_id}` references unknown shared workspace `{shared}`"
                ))
            })?;
            requested.push(path.clone());
        }
    }
    Ok(requested)
}

pub(crate) fn normalize_absolute_path(path: &Path) -> Result<PathBuf, OrchestratorError> {
    if !path.is_absolute() {
        return Err(OrchestratorError::WorkspacePathValidation {
            path: path.display().to_string(),
            reason: "path must be absolute".to_string(),
        });
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(v) => normalized.push(v),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(OrchestratorError::WorkspacePathValidation {
                        path: path.display().to_string(),
                        reason: "path escapes filesystem root".to_string(),
                    });
                }
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }
    Ok(normalized)
}

fn io_error(path: &Path, source: std::io::Error) -> OrchestratorError {
    OrchestratorError::Io {
        path: path.display().to_string(),
        source,
    }
}
