use crate::app::command_support::ensure_runtime_root;
use crate::runtime::load_supervisor_state;
use std::fs;

pub fn cmd_attach() -> Result<String, String> {
    let paths = ensure_runtime_root()?;
    let state = load_supervisor_state(&paths).map_err(|e| e.to_string())?;
    if state.running {
        return Ok("attached=true\nsummary=connected to supervisor runtime".to_string());
    }

    let runs_dir = paths.root.join("workflows/runs");
    let mut count = 0usize;
    if runs_dir.exists() {
        for entry in fs::read_dir(&runs_dir)
            .map_err(|e| format!("failed to read {}: {e}", runs_dir.display()))?
        {
            let path = entry
                .map_err(|e| format!("failed to read workflow entry: {e}"))?
                .path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                count += 1;
            }
        }
    }

    Ok(format!("attached=false\nsummary=workflow_runs={count}"))
}
