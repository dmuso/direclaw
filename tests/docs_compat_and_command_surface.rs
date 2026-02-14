use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_direclaw"))
        .args(args)
        .output()
        .expect("run direclaw")
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn docs_referenced_example_paths_exist() {
    for relative in [
        "docs/build/spec/examples/settings/minimal.settings.yaml",
        "docs/build/spec/examples/settings/full.settings.yaml",
        "docs/build/spec/examples/orchestrators/minimal.orchestrator.yaml",
        "docs/build/spec/examples/orchestrators/engineering.orchestrator.yaml",
        "docs/build/spec/examples/orchestrators/product.orchestrator.yaml",
        "docs/build/compatibility-notes.md",
    ] {
        assert!(
            project_root().join(relative).exists(),
            "missing documented path: {relative}"
        );
    }
}

#[test]
fn specs_reference_current_config_paths_and_supported_command_surface() {
    for spec in [
        "docs/build/spec/01-runtime-filesystem.md",
        "docs/build/spec/02-queue-processing.md",
        "docs/build/spec/05-workflow-orchestration.md",
        "docs/build/spec/09-configuration-cli.md",
    ] {
        let text = read(&project_root().join(spec));
        assert!(
            !text.contains("~/.direclaw.yaml"),
            "spec still references legacy config path: {spec}"
        );
        assert!(
            text.contains("~/.direclaw/config.yaml"),
            "spec should reference ~/.direclaw/config.yaml: {spec}"
        );
    }

    let config_cli = read(&project_root().join("docs/build/spec/09-configuration-cli.md"));
    for command in [
        "orchestrator list",
        "orchestrator add",
        "orchestrator show",
        "orchestrator remove",
        "workflow list",
        "workflow show",
        "workflow run",
        "workflow status",
        "workflow progress",
        "workflow cancel",
        "orchestrator-agent list",
        "orchestrator-agent add",
        "orchestrator-agent show",
        "orchestrator-agent remove",
        "orchestrator-agent reset",
        "channel-profile list",
        "channel-profile add",
        "channel-profile show",
        "channel-profile remove",
        "channel-profile set-orchestrator",
    ] {
        assert!(
            config_cli.contains(command),
            "spec command missing from docs/build/spec/09-configuration-cli.md: {command}"
        );
    }

    for (args, expected) in [
        (
            &["orchestrator"][..],
            "usage: orchestrator <list|add|show|remove|set-private-workspace|grant-shared-access|revoke-shared-access|set-selector-agent|set-default-workflow|set-selection-max-retries> ...",
        ),
        (
            &["workflow"][..],
            "usage: workflow <list|show|add|remove|run|status|progress|cancel> ...",
        ),
        (
            &["orchestrator-agent"][..],
            "usage: orchestrator-agent <list|add|show|remove|reset> ...",
        ),
        (
            &["channel-profile"][..],
            "usage: channel-profile <list|add|show|remove|set-orchestrator> ...",
        ),
    ] {
        let output = run(args);
        let text = output_text(&output);
        assert!(
            text.contains(expected),
            "expected `{expected}` in CLI output, got:\n{text}"
        );
    }
}
