use direclaw::config::load_global_settings;
use direclaw::runtime::{bootstrap_state_root, default_state_root_path, StatePaths};

fn run() -> Result<(), String> {
    let settings =
        load_global_settings().map_err(|err| format!("failed to load ~/.direclaw.yaml: {err}"))?;
    let state_root = default_state_root_path()
        .map_err(|err| format!("failed to resolve default state root: {err}"))?;
    let paths = StatePaths::new(&state_root);
    bootstrap_state_root(&paths).map_err(|err| {
        format!(
            "failed to bootstrap state root {}: {err}",
            state_root.display()
        )
    })?;

    println!("direclaw initialized");
    println!("global_config={}", paths.settings_file().display());
    println!("state_root={}", state_root.display());
    println!("workspace_path={}", settings.workspace_path.display());
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
