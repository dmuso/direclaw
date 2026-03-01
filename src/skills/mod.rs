use crate::config::ConfigError;
use std::fs;
use std::path::Path;

const SKILL_FILES: &[(&str, &str)] = &[
    (
        "direclaw-routing-selector-forensics/SKILL.md",
        include_str!("assets/direclaw-routing-selector-forensics.md"),
    ),
    (
        "direclaw-workflow-run-forensics/SKILL.md",
        include_str!("assets/direclaw-workflow-run-forensics.md"),
    ),
    (
        "direclaw-scheduler-cron-management/SKILL.md",
        include_str!("assets/direclaw-scheduler-cron-management.md"),
    ),
    (
        "direclaw-log-investigation/SKILL.md",
        include_str!("assets/direclaw-log-investigation.md"),
    ),
    (
        "direclaw-agent-management/SKILL.md",
        include_str!("assets/direclaw-agent-management.md"),
    ),
    (
        "direclaw-skill-management/SKILL.md",
        include_str!("assets/direclaw-skill-management.md"),
    ),
    (
        "direclaw-workflow-authoring/SKILL.md",
        include_str!("assets/direclaw-workflow-authoring.md"),
    ),
    (
        "direclaw-prompt-contracts/SKILL.md",
        include_str!("assets/direclaw-prompt-contracts.md"),
    ),
];

fn io_create_dir_error(path: &Path, source: std::io::Error) -> ConfigError {
    ConfigError::CreateDir {
        path: path.display().to_string(),
        source,
    }
}

fn io_write_error(path: &Path, source: std::io::Error) -> ConfigError {
    ConfigError::Write {
        path: path.display().to_string(),
        source,
    }
}

fn ensure_file(path: &Path, body: &str) -> Result<(), ConfigError> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| io_create_dir_error(parent, source))?;
    }
    fs::write(path, body).map_err(|source| io_write_error(path, source))
}

pub fn ensure_orchestrator_skill_files(private_workspace: &Path) -> Result<(), ConfigError> {
    let skills_root = private_workspace.join("skills");
    fs::create_dir_all(&skills_root).map_err(|source| io_create_dir_error(&skills_root, source))?;

    for (rel_path, body) in SKILL_FILES {
        ensure_file(&skills_root.join(rel_path), body)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_orchestrator_skill_files_seeds_builtins() {
        let temp = tempdir().expect("tempdir");

        ensure_orchestrator_skill_files(temp.path()).expect("seed skills");

        for (rel_path, body) in SKILL_FILES {
            let path = temp.path().join("skills").join(rel_path);
            assert!(path.is_file(), "missing skill file {}", path.display());
            let actual = fs::read_to_string(path).expect("read skill file");
            assert_eq!(actual, *body, "skill content mismatch for {rel_path}");
        }
    }
}
