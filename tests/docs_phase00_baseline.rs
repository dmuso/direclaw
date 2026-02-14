use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn walk_markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("failed to read dir {}: {e}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|e| panic!("failed to read entry: {e}"));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            {
                out.push(path);
            }
        }
    }

    out.sort();
    out
}

fn extract_markdown_links(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut idx = 0;
    while let Some(start) = content[idx..].find("](") {
        let link_start = idx + start + 2;
        let Some(close_rel) = content[link_start..].find(')') else {
            break;
        };
        let raw = &content[link_start..link_start + close_rel];
        let candidate = raw
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches('<')
            .trim_matches('>')
            .to_string();
        links.push(candidate);
        idx = link_start + close_rel + 1;
    }
    links
}

fn phase_task_files() -> Vec<PathBuf> {
    let root = repo_root().join("docs/build/tasks");
    let mut files: Vec<PathBuf> = fs::read_dir(&root)
        .expect("read tasks dir")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_str()?;
            if name.starts_with("phase-") && name.ends_with(".md") && !name.ends_with("-review.md")
            {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    files.sort();
    files
}

fn normalize_reference_token(value: &str) -> &str {
    value.trim().trim_matches('`')
}

fn is_valid_test_id_reference(reference: &str) -> bool {
    let Some((path, test_id)) = reference.split_once("::") else {
        return false;
    };
    if !path.starts_with("tests/") || !path.ends_with(".rs") {
        return false;
    }
    if test_id.is_empty() {
        return false;
    }
    path.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-' | '.'))
        && test_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_valid_planned_reference(reference: &str) -> bool {
    let Some(stable_id) = reference.strip_prefix("planned:") else {
        return false;
    };
    !stable_id.is_empty()
        && stable_id.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')
        })
}

#[test]
fn v1_scope_docs_are_slack_only_and_deferred_channels_are_roadmapped() {
    let root = repo_root();
    let readme = read(&root.join("README.md"));
    assert!(
        readme.contains("## v1 Scope") && readme.contains("Slack as the only channel adapter"),
        "README must declare Slack-only v1 scope"
    );
    assert!(
        readme.contains("## Deferred After v1"),
        "README must include Deferred After v1 section"
    );

    for term in ["Discord", "Telegram", "WhatsApp"] {
        let count = readme.matches(term).count();
        assert_eq!(
            count, 1,
            "README should mention {term} only once in deferred section; found {count}"
        );
    }

    let user_guide_readme = read(&root.join("docs/user-guide/README.md"));
    assert!(
        user_guide_readme.contains("DireClaw v1 supports Slack only."),
        "user guide README must declare Slack-only scope"
    );
    assert!(
        user_guide_readme.contains("## Deferred After v1"),
        "user guide README must include Deferred After v1"
    );

    let guide_files = walk_markdown_files(&root.join("docs/user-guide"));
    for file in guide_files {
        if file.ends_with(Path::new("docs/user-guide/README.md")) {
            continue;
        }
        let content = read(&file).to_ascii_lowercase();
        for forbidden in ["discord", "telegram", "whatsapp"] {
            assert!(
                !content.contains(forbidden),
                "{} should not claim/support non-v1 channel `{forbidden}`",
                file.display()
            );
        }
    }

    let adapter_spec = read(&root.join("docs/build/spec/07-channel-adapters.md"));
    assert!(
        adapter_spec.contains("DireClaw v1 supports Slack only."),
        "channel adapter spec must declare Slack-only v1 scope"
    );
    assert!(
        adapter_spec.contains("Discord, Telegram, and WhatsApp are deferred after v1"),
        "channel adapter spec must explicitly defer non-Slack adapters post-v1"
    );

    let reliability_spec = read(&root.join("docs/build/spec/12-reliability-compat-testing.md"));
    assert!(
        reliability_spec.contains("DireClaw v1 channel scope is Slack-only."),
        "reliability spec must state Slack-only v1 channel scope"
    );
    assert!(
        reliability_spec.contains("Discord, Telegram, and WhatsApp compatibility and adapter testing are deferred after v1."),
        "reliability spec must explicitly defer non-Slack adapter testing post-v1"
    );
}

#[test]
fn markdown_links_and_paths_resolve_for_contributor_and_operator_docs() {
    let root = repo_root();

    let mut files = vec![root.join("README.md"), root.join("AGENTS.md")];
    files.extend(walk_markdown_files(&root.join("docs/user-guide")));
    files.extend(walk_markdown_files(&root.join("docs/build/spec")));
    files.extend(phase_task_files());
    files.push(root.join("docs/build/README.tasks.md"));
    files.push(root.join("docs/build/release-readiness-plan.md"));

    for file in files {
        let content = read(&file);
        assert!(
            !content.contains("docs/spec/"),
            "{} still references non-canonical docs/spec path",
            file.display()
        );

        for link in extract_markdown_links(&content) {
            if link.is_empty()
                || link.starts_with('#')
                || link.starts_with("http://")
                || link.starts_with("https://")
                || link.starts_with("mailto:")
            {
                continue;
            }
            let target = link.split('#').next().unwrap_or("");
            if target.is_empty() {
                continue;
            }
            let resolved = file
                .parent()
                .expect("doc parent")
                .join(target)
                .components()
                .collect::<PathBuf>();
            assert!(
                resolved.exists(),
                "{} links to missing path {}",
                file.display(),
                resolved.display()
            );
        }
    }
}

#[test]
fn release_blocking_requirements_have_traceability_rows_with_task_and_test_ownership() {
    let root = repo_root();
    let plan = read(&root.join("docs/build/release-readiness-plan.md"));
    let traceability = read(&root.join("docs/build/review/requirement-traceability.md"));

    let mut in_section = false;
    let mut requirements = Vec::new();
    for line in plan.lines() {
        if line.trim() == "## Release Acceptance Criteria (Go/No-Go)" {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with("## ") {
            break;
        }
        if in_section {
            let trimmed = line.trim();
            let mut chars = trimmed.chars();
            if let (Some(first), Some('.')) = (chars.next(), chars.next()) {
                if first.is_ascii_digit() {
                    requirements.push(trimmed.to_string());
                }
            }
        }
    }

    assert!(
        !requirements.is_empty(),
        "failed to find numbered release acceptance criteria in plan"
    );

    let mut rows: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in traceability.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("| RB-") {
            continue;
        }
        let columns: Vec<String> = trimmed
            .split('|')
            .map(|col| col.trim().to_string())
            .filter(|col| !col.is_empty())
            .collect();
        assert!(
            columns.len() >= 4,
            "traceability row must contain id, requirement, tasks, tests: {trimmed}"
        );
        rows.insert(columns[0].clone(), columns);
    }

    assert_eq!(
        rows.len(),
        requirements.len(),
        "traceability rows must match release requirement count"
    );

    let known_task_ids: BTreeSet<String> = phase_task_files()
        .into_iter()
        .flat_map(|path| {
            read(&path)
                .lines()
                .filter_map(|line| line.trim().strip_prefix("### "))
                .filter_map(|heading| heading.split_whitespace().next())
                .filter(|id| {
                    let bytes = id.as_bytes();
                    bytes.len() == 7 && bytes[0] == b'P' && bytes[3] == b'-' && bytes[4] == b'T'
                })
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        })
        .collect();

    for idx in 1..=requirements.len() {
        let id = format!("RB-{idx}");
        let row = rows
            .get(&id)
            .unwrap_or_else(|| panic!("missing traceability row for {id}"));
        let tasks = &row[2];
        let tests = &row[3];
        assert!(
            !tasks.is_empty() && !tasks.eq_ignore_ascii_case("tbd"),
            "{id} must map to owning tasks"
        );
        assert!(
            !tests.is_empty() && !tests.eq_ignore_ascii_case("tbd"),
            "{id} must map to test references"
        );
        for token in tests.split(';').map(normalize_reference_token) {
            if token.is_empty() {
                continue;
            }
            assert!(
                is_valid_test_id_reference(token) || is_valid_planned_reference(token),
                "{id} includes invalid test reference token `{token}`; use `tests/<file>.rs::<test_id>` or `planned:<stable_id>`"
            );
        }

        for task in tasks.split(',').map(|v| v.trim()) {
            if task.is_empty() {
                continue;
            }
            assert!(
                known_task_ids.contains(task),
                "{id} references unknown task id `{task}`"
            );
        }
    }
}

#[test]
fn every_phase_task_section_has_required_status_acceptance_and_automated_test_blocks() {
    for file in phase_task_files() {
        let content = read(&file);
        let mut saw_task = false;

        for section in content.split("\n### ").skip(1) {
            saw_task = true;
            assert!(
                section.contains("- Status: `"),
                "{} task section is missing status",
                file.display()
            );
            assert!(
                section.contains("- Acceptance Criteria:"),
                "{} task section is missing Acceptance Criteria",
                file.display()
            );
            assert!(
                section.contains("- Automated Test Requirements:"),
                "{} task section is missing Automated Test Requirements",
                file.display()
            );

            let status_line = section
                .lines()
                .find(|line| line.trim_start().starts_with("- Status:"))
                .unwrap_or_else(|| panic!("{} missing status line", file.display()));
            let token = status_line
                .split('`')
                .nth(1)
                .unwrap_or_else(|| panic!("{} invalid status token format", file.display()));
            assert!(
                matches!(token, "todo" | "in_progress" | "complete"),
                "{} has invalid status token `{token}`",
                file.display()
            );
        }

        assert!(
            saw_task,
            "{} should contain at least one task section",
            file.display()
        );
    }
}
