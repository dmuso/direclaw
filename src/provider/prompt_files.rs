use crate::provider::{io_error, PromptArtifacts, ProviderError};
use std::fs;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetResolution {
    pub reset_requested: bool,
    pub consumed_agent: bool,
}

pub fn consume_reset_flag(agent_flag: &Path) -> Result<ResetResolution, ProviderError> {
    let mut consumed_agent = false;

    if agent_flag.exists() {
        fs::remove_file(agent_flag).map_err(|err| io_error(agent_flag, err))?;
        consumed_agent = true;
    }

    Ok(ResetResolution {
        reset_requested: consumed_agent,
        consumed_agent,
    })
}

pub fn write_file_backed_prompt(
    workspace: &Path,
    _request_id: &str,
    prompt: &str,
    context: &str,
) -> Result<PromptArtifacts, ProviderError> {
    fs::create_dir_all(workspace).map_err(|err| io_error(workspace, err))?;
    let prompt_file = workspace.join("prompt.md");
    let context_file = workspace.join("context.md");

    fs::write(&prompt_file, prompt).map_err(|err| io_error(&prompt_file, err))?;
    fs::write(&context_file, context).map_err(|err| io_error(&context_file, err))?;

    Ok(PromptArtifacts {
        prompt_file,
        context_files: vec![context_file],
    })
}

pub fn read_to_string(path: &Path) -> Result<String, ProviderError> {
    let mut file = fs::File::open(path).map_err(|err| io_error(path, err))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|err| io_error(path, err))?;
    Ok(buf)
}
