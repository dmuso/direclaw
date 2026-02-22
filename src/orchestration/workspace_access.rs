use crate::config::{OrchestratorConfig, Settings};
use crate::orchestration::error::OrchestratorError;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceAccessContext {
    pub orchestrator_id: String,
    pub private_workspace_root: PathBuf,
    pub shared_workspaces: BTreeMap<String, SharedWorkspaceAccess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedWorkspaceAccess {
    pub root: PathBuf,
    pub description: String,
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

    let mut shared_workspaces = BTreeMap::new();
    for grant in &orchestrator.shared_access {
        let shared = settings.shared_workspaces.get(grant).ok_or_else(|| {
            OrchestratorError::Config(format!(
                "orchestrator `{orchestrator_id}` references unknown shared workspace `{grant}`"
            ))
        })?;
        let normalized = canonicalize_absolute_path_if_exists(&shared.path)?;
        shared_workspaces.insert(
            grant.clone(),
            SharedWorkspaceAccess {
                root: normalized,
                description: shared.description.clone(),
            },
        );
    }

    Ok(WorkspaceAccessContext {
        orchestrator_id: orchestrator_id.to_string(),
        private_workspace_root: private_workspace,
        shared_workspaces,
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
            .shared_workspaces
            .values()
            .any(|shared| normalized.starts_with(&shared.root))
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

fn canonicalize_absolute_path_if_exists(path: &Path) -> Result<PathBuf, OrchestratorError> {
    match fs::canonicalize(path) {
        Ok(canonical) => normalize_absolute_path(&canonical),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => normalize_absolute_path(path),
        Err(err) => Err(io_error(path, err)),
    }
}

fn collect_orchestrator_requested_paths(
    context: &WorkspaceAccessContext,
    _settings: &Settings,
    _orchestrator: &OrchestratorConfig,
) -> Result<Vec<PathBuf>, OrchestratorError> {
    Ok(vec![context.private_workspace_root.clone()])
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
