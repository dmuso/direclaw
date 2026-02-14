use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::tempdir;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn write(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap_or_else(|e| panic!("failed to write {}: {e}", path.display()));
}

fn run_gate(
    artifacts_dir: &Path,
    tests_marker: &Path,
    docs_marker: &Path,
    traceability_file: &Path,
    checklist_file: &Path,
    release_tag: &str,
) -> Output {
    Command::new("bash")
        .arg(repo_root().join("scripts/ci/release-gate.sh"))
        .arg("--artifacts-dir")
        .arg(artifacts_dir)
        .arg("--tests-marker")
        .arg(tests_marker)
        .arg("--docs-marker")
        .arg(docs_marker)
        .arg("--traceability-file")
        .arg(traceability_file)
        .arg("--checklist-file")
        .arg(checklist_file)
        .arg("--release-tag")
        .arg(release_tag)
        .output()
        .expect("run release gate script")
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_err_contains(output: &Output, needle: &str) {
    assert!(
        !output.status.success(),
        "expected failure, stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        text.contains(needle),
        "expected output to contain `{needle}`, got:\n{text}"
    );
}

fn seed_success_inputs(root: &Path, tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let artifacts = root.join("dist");
    fs::create_dir_all(&artifacts).expect("create artifacts dir");

    for target in [
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
    ] {
        write(
            &artifacts.join(format!("direclaw-{tag}-{target}.tar.gz")),
            "artifact",
        );
    }
    write(
        &artifacts.join("checksums.txt"),
        "deadbeef  artifact.tar.gz\n",
    );

    let tests_marker = root.join("tests.ok");
    let docs_marker = root.join("docs.ok");
    write(&tests_marker, "ok\n");
    write(&docs_marker, "ok\n");

    let traceability = root.join("traceability.md");
    write(
        &traceability,
        "| RB-1 | req | task | `tests/a.rs::x` |\n| RB-2 | req | task | `tests/b.rs::y` |\n",
    );

    let checklist = root.join("checklist.md");
    write(&checklist, "RB-1\nRB-2\nRB-3\nRB-4\nRB-5\nRB-6\nRB-7\n");

    (
        artifacts,
        tests_marker,
        docs_marker,
        traceability,
        checklist,
    )
}

#[test]
fn release_gate_passes_with_all_blockers_satisfied() {
    let temp = tempdir().expect("tempdir");
    let (artifacts, tests_marker, docs_marker, traceability, checklist) =
        seed_success_inputs(temp.path(), "v1.0.0");

    let output = run_gate(
        &artifacts,
        &tests_marker,
        &docs_marker,
        &traceability,
        &checklist,
        "v1.0.0",
    );
    assert_ok(&output);
}

#[test]
fn release_gate_fails_when_any_blocker_is_violated() {
    let temp = tempdir().expect("tempdir");
    let (artifacts, tests_marker, docs_marker, traceability, checklist) =
        seed_success_inputs(temp.path(), "v1.0.0");

    fs::remove_file(&tests_marker).expect("remove tests marker");
    let missing_tests = run_gate(
        &artifacts,
        &tests_marker,
        &docs_marker,
        &traceability,
        &checklist,
        "v1.0.0",
    );
    assert_err_contains(&missing_tests, "tests gate marker missing or empty");
    write(&tests_marker, "ok\n");

    fs::remove_file(&docs_marker).expect("remove docs marker");
    let missing_docs = run_gate(
        &artifacts,
        &tests_marker,
        &docs_marker,
        &traceability,
        &checklist,
        "v1.0.0",
    );
    assert_err_contains(&missing_docs, "docs gate marker missing or empty");
    write(&docs_marker, "ok\n");

    fs::remove_file(artifacts.join("direclaw-v1.0.0-aarch64-apple-darwin.tar.gz"))
        .expect("remove artifact");
    let missing_artifact = run_gate(
        &artifacts,
        &tests_marker,
        &docs_marker,
        &traceability,
        &checklist,
        "v1.0.0",
    );
    assert_err_contains(
        &missing_artifact,
        "missing artifact for target aarch64-apple-darwin",
    );
    write(
        &artifacts.join("direclaw-v1.0.0-aarch64-apple-darwin.tar.gz"),
        "artifact",
    );

    write(
        &traceability,
        "| RB-1 | req | task | planned:test.todo |\n| RB-2 | req | task | `tests/b.rs::y` |\n",
    );
    let placeholder_traceability = run_gate(
        &artifacts,
        &tests_marker,
        &docs_marker,
        &traceability,
        &checklist,
        "v1.0.0",
    );
    assert_err_contains(
        &placeholder_traceability,
        "traceability still contains planned placeholder references",
    );
}
