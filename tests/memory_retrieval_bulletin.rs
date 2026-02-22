use direclaw::memory::{
    build_memory_bulletin, generate_bulletin_for_message, hybrid_recall, query_full_text,
    query_vector, BulletinSectionName, HybridRecallRequest, HybridRecallResultMode,
    MemoryBulletinOptions, MemoryCapturedBy, MemoryNode, MemoryNodeType, MemoryPaths,
    MemoryRecallError, MemoryRecallOptions, MemoryRepository, MemorySource, MemorySourceRecord,
    MemorySourceType, MemoryStatus, VectorQueryOutcome,
};
use direclaw::orchestration::workspace_access::WorkspaceAccessContext;
use rusqlite::params;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().expect("canonicalize")
}

fn parse_memory_log_events(path: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .expect("read memory log")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("memory log line must be json"))
        .collect()
}

fn approx_eq(left: f64, right: f64) {
    let delta = (left - right).abs();
    assert!(
        delta < 1e-9,
        "expected {left} to be approximately equal to {right} (delta={delta})"
    );
}

fn source_record(
    orchestrator_id: &str,
    key: &str,
    source_path: Option<PathBuf>,
) -> MemorySourceRecord {
    MemorySourceRecord {
        orchestrator_id: orchestrator_id.to_string(),
        idempotency_key: key.to_string(),
        source_type: MemorySourceType::Manual,
        source_path,
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::System,
    }
}

#[allow(clippy::too_many_arguments)]
fn memory_node(
    memory_id: &str,
    orchestrator_id: &str,
    node_type: MemoryNodeType,
    content: &str,
    summary: &str,
    importance: u8,
    confidence: f32,
    updated_at: i64,
    source_path: Option<PathBuf>,
    conversation_id: Option<&str>,
) -> MemoryNode {
    MemoryNode {
        memory_id: memory_id.to_string(),
        orchestrator_id: orchestrator_id.to_string(),
        node_type,
        importance,
        content: content.to_string(),
        summary: summary.to_string(),
        confidence,
        source: MemorySource {
            source_type: MemorySourceType::Manual,
            source_path,
            conversation_id: conversation_id.map(|value| value.to_string()),
            workflow_run_id: None,
            step_id: None,
            captured_by: MemoryCapturedBy::System,
        },
        status: MemoryStatus::Active,
        created_at: updated_at,
        updated_at,
    }
}

fn setup_repo() -> (tempfile::TempDir, MemoryRepository, MemoryPaths) {
    let dir = tempdir().expect("tempdir");
    let runtime_root = dir.path().join("runtime");
    fs::create_dir_all(&runtime_root).expect("runtime root");
    let paths = MemoryPaths::from_runtime_root(&runtime_root);
    fs::create_dir_all(&paths.root).expect("memory root");
    let repo = MemoryRepository::open(&paths.database, "alpha").expect("open repo");
    repo.ensure_schema().expect("schema");
    (dir, repo, paths)
}

#[test]
fn full_text_adapter_returns_ranked_results_with_provenance() {
    let (dir, repo, _) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let nodes = vec![
        memory_node(
            "mem-1",
            "alpha",
            MemoryNodeType::Fact,
            "wizard launch plan",
            "wizard",
            50,
            0.8,
            10,
            Some(canonical(&src)),
            Some("conv-a"),
        ),
        memory_node(
            "mem-2",
            "alpha",
            MemoryNodeType::Fact,
            "garden schedule",
            "garden",
            30,
            0.7,
            11,
            Some(canonical(&src)),
            Some("conv-b"),
        ),
    ];

    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k1", Some(canonical(&src))),
        &nodes,
        &[],
    )
    .expect("persist");

    let hits = query_full_text(&repo, "wizard", 10).expect("full text query");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.memory_id, "mem-1");
    assert_eq!(
        hits[0].provenance.conversation_id.as_deref(),
        Some("conv-a")
    );
    assert_eq!(
        hits[0].provenance.source_path.as_deref(),
        Some(canonical(&src).as_path())
    );
}

#[test]
fn full_text_adapter_handles_punctuation_without_fts_parse_errors() {
    let (dir, repo, _) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let node = memory_node(
        "mem-punct",
        "alpha",
        MemoryNodeType::Fact,
        "deployment failed due to flaky tests",
        "deployment failed",
        50,
        0.8,
        10,
        Some(canonical(&src)),
        Some("conv-a"),
    );
    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k-punct", Some(canonical(&src))),
        &[node],
        &[],
    )
    .expect("persist");

    let hits = query_full_text(&repo, "why did deployment fail, and where?", 10)
        .expect("full text query must not fail on punctuation");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.memory_id, "mem-punct");
}

#[test]
fn bulletin_prefers_query_relevant_sentence_from_long_memory_content() {
    let (dir, repo, paths) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let node = memory_node(
        "mem-long",
        "alpha",
        MemoryNodeType::Fact,
        "This paragraph starts with setup and background that does not answer the question. \
         It keeps going with details that are mostly irrelevant to deployment causes. \
         Root cause: deployment failed because migrations were executed out of order.",
        "background details about deployment context",
        65,
        0.9,
        10,
        Some(canonical(&src)),
        Some("conv-a"),
    );
    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k-long", Some(canonical(&src))),
        &[node],
        &[],
    )
    .expect("persist");

    let recall = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: Some("conv-a".to_string()),
            query_text: "why did deployment fail".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        None,
        &paths.log_file,
    )
    .expect("hybrid recall");

    let bulletin = build_memory_bulletin(
        &recall,
        &MemoryBulletinOptions {
            max_chars: 4_000,
            generated_at: 10,
        },
    );
    assert!(
        bulletin.rendered.contains("Root cause: deployment failed"),
        "expected bulletin to include query-relevant sentence, got: {}",
        bulletin.rendered
    );
}

#[test]
fn vector_adapter_reports_missing_embeddings_and_hybrid_recall_falls_back_to_text() {
    let (dir, repo, paths) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let node = memory_node(
        "mem-1",
        "alpha",
        MemoryNodeType::Fact,
        "wizard launch plan",
        "wizard",
        50,
        0.8,
        10,
        Some(canonical(&src)),
        Some("conv-a"),
    );
    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k1", Some(canonical(&src))),
        &[node],
        &[],
    )
    .expect("persist");

    let vector_out = query_vector(&repo, None, 10).expect("vector query");
    assert_eq!(
        vector_out,
        VectorQueryOutcome::UnavailableMissingQueryEmbedding
    );

    let recall = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: Some("conv-other".to_string()),
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        None,
        &paths.log_file,
    )
    .expect("hybrid recall");

    assert_eq!(recall.mode, HybridRecallResultMode::FullTextOnly);
    assert_eq!(recall.memories.len(), 1);
    assert_eq!(recall.memories[0].memory.memory_id, "mem-1");
}

#[test]
fn rrf_merge_and_scoring_modifiers_are_deterministic() {
    let (_dir, repo, paths) = setup_repo();

    let result_a = HybridRecallResultMode::Hybrid;
    let result_b = HybridRecallResultMode::Hybrid;
    assert_eq!(result_a, result_b);

    let recall_1 = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "nothing".to_string(),
            query_embedding: Some(vec![0.1, 0.2, 0.3]),
        },
        &MemoryRecallOptions {
            top_n: 20,
            rrf_k: 60,
            top_k_text: 50,
            top_k_vector: 50,
            now_unix_seconds: 1_800_000_000,
        },
        None,
        &paths.log_file,
    )
    .expect("recall 1");

    let recall_2 = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "nothing".to_string(),
            query_embedding: Some(vec![0.1, 0.2, 0.3]),
        },
        &MemoryRecallOptions {
            top_n: 20,
            rrf_k: 60,
            top_k_text: 50,
            top_k_vector: 50,
            now_unix_seconds: 1_800_000_000,
        },
        None,
        &paths.log_file,
    )
    .expect("recall 2");

    let ids_1 = recall_1
        .memories
        .iter()
        .map(|entry| entry.memory.memory_id.clone())
        .collect::<Vec<_>>();
    let ids_2 = recall_2
        .memories
        .iter()
        .map(|entry| entry.memory.memory_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(ids_1, ids_2);
}

#[test]
fn hybrid_recall_enforces_top_n_and_includes_citation_ready_metadata() {
    let (dir, repo, paths) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let nodes = vec![
        memory_node(
            "mem-a",
            "alpha",
            MemoryNodeType::Goal,
            "ship wizard",
            "ship wizard",
            95,
            1.0,
            100,
            Some(canonical(&src)),
            Some("conv-a"),
        ),
        memory_node(
            "mem-b",
            "alpha",
            MemoryNodeType::Todo,
            "draft checklist",
            "checklist",
            80,
            1.0,
            99,
            Some(canonical(&src)),
            Some("conv-a"),
        ),
        memory_node(
            "mem-c",
            "alpha",
            MemoryNodeType::Fact,
            "wizard notes",
            "notes",
            70,
            1.0,
            98,
            Some(canonical(&src)),
            Some("conv-a"),
        ),
    ];

    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k1", Some(canonical(&src))),
        &nodes,
        &[],
    )
    .expect("persist");

    let recall = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions {
            top_n: 2,
            ..MemoryRecallOptions::default()
        },
        None,
        &paths.log_file,
    )
    .expect("recall");

    assert_eq!(recall.memories.len(), 2);
    for item in &recall.memories {
        assert!(!item.citation.memory_id.trim().is_empty());
        assert!(!item.citation.source_type.trim().is_empty());
    }
}

#[test]
fn recall_scope_denies_cross_orchestrator_and_logs() {
    let (_dir, repo, paths) = setup_repo();

    let err = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "beta".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        None,
        &paths.log_file,
    )
    .expect_err("cross orchestrator should fail");

    assert!(matches!(
        err,
        MemoryRecallError::CrossOrchestratorDenied { .. }
    ));
    let events = parse_memory_log_events(&paths.log_file);
    assert!(events.iter().any(|event| {
        event["event"] == "memory.recall.scope_denied"
            && event["requested_orchestrator_id"] == "beta"
            && event["available_orchestrator_id"] == "alpha"
    }));
}

#[test]
fn recall_enforces_source_path_workspace_access() {
    let (dir, repo, paths) = setup_repo();
    let private_root = dir.path().join("private");
    fs::create_dir_all(&private_root).expect("private root");
    let allowed_src = private_root.join("allowed.txt");
    let denied_src = dir.path().join("outside.txt");
    fs::write(&allowed_src, "x").expect("write allowed");
    fs::write(&denied_src, "x").expect("write denied");

    let nodes = vec![
        memory_node(
            "mem-allowed",
            "alpha",
            MemoryNodeType::Fact,
            "wizard allowed",
            "allowed",
            80,
            0.9,
            10,
            Some(canonical(&allowed_src)),
            None,
        ),
        memory_node(
            "mem-denied",
            "alpha",
            MemoryNodeType::Fact,
            "wizard denied",
            "denied",
            80,
            0.9,
            9,
            Some(canonical(&denied_src)),
            None,
        ),
    ];

    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k1", Some(canonical(&allowed_src))),
        &nodes,
        &[],
    )
    .expect("persist");

    let context = WorkspaceAccessContext {
        orchestrator_id: "alpha".to_string(),
        private_workspace_root: canonical(&private_root),
        shared_workspaces: BTreeMap::new(),
    };

    let err = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        Some(&context),
        &paths.log_file,
    )
    .expect_err("source path access should fail");

    assert!(matches!(
        err,
        MemoryRecallError::SourcePathAccessDenied { .. }
    ));
    let denied_path = canonical(&denied_src).display().to_string();
    let events = parse_memory_log_events(&paths.log_file);
    assert!(events.iter().any(|event| {
        event["event"] == "memory.recall.source_path_denied" && event["path"] == denied_path
    }));
}

#[test]
fn bulletin_includes_required_sections_citations_and_deterministic_truncation_priority() {
    let (dir, repo, paths) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let nodes = vec![
        memory_node(
            "goal-1",
            "alpha",
            MemoryNodeType::Goal,
            "goal goal goal goal",
            "goal",
            95,
            1.0,
            100,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "todo-1",
            "alpha",
            MemoryNodeType::Todo,
            "todo todo todo todo",
            "todo",
            90,
            1.0,
            99,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "decision-1",
            "alpha",
            MemoryNodeType::Decision,
            "decision decision decision",
            "decision",
            88,
            1.0,
            98,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "fact-1",
            "alpha",
            MemoryNodeType::Fact,
            "fact fact fact fact",
            "fact",
            10,
            0.5,
            97,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "fact-2",
            "alpha",
            MemoryNodeType::Fact,
            "fact extra extra extra",
            "fact-2",
            10,
            0.5,
            96,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "fact-3",
            "alpha",
            MemoryNodeType::Fact,
            "fact another another another",
            "fact-3",
            10,
            0.5,
            95,
            Some(canonical(&src)),
            None,
        ),
    ];

    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k1", Some(canonical(&src))),
        &nodes,
        &[],
    )
    .expect("persist");

    let recall = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "goal OR todo OR decision OR fact".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        None,
        &paths.log_file,
    )
    .expect("recall");

    let bulletin_a = build_memory_bulletin(
        &recall,
        &MemoryBulletinOptions {
            max_chars: 320,
            generated_at: 1_800_000_100,
        },
    );
    let bulletin_b = build_memory_bulletin(
        &recall,
        &MemoryBulletinOptions {
            max_chars: 320,
            generated_at: 1_800_000_100,
        },
    );

    assert_eq!(bulletin_a.rendered, bulletin_b.rendered);
    assert!(bulletin_a
        .sections
        .iter()
        .any(|section| section.name == BulletinSectionName::KnowledgeSummary));
    assert!(bulletin_a
        .sections
        .iter()
        .any(|section| section.name == BulletinSectionName::ActiveGoals));
    assert!(bulletin_a
        .sections
        .iter()
        .any(|section| section.name == BulletinSectionName::OpenTodos));
    assert!(bulletin_a
        .sections
        .iter()
        .any(|section| section.name == BulletinSectionName::RecentDecisions));
    assert!(bulletin_a
        .citations
        .iter()
        .any(|citation| citation.memory_id == "goal-1"));
    assert!(bulletin_a
        .citations
        .iter()
        .any(|citation| citation.memory_id == "todo-1"));
    assert!(bulletin_a
        .citations
        .iter()
        .any(|citation| citation.memory_id == "decision-1"));
}

#[test]
fn bulletin_generation_falls_back_to_previous_snapshot_or_empty_payload() {
    let (_dir, repo, paths) = setup_repo();
    fs::create_dir_all(&paths.bulletins).expect("bulletins dir");

    let previous = paths.bulletins.join("msg-prev.json");
    fs::write(
        &previous,
        r#"{"rendered":"previous-bulletin","citations":[],"sections":[],"generatedAt":1}"#,
    )
    .expect("write previous bulletin");

    let fallback = generate_bulletin_for_message(
        &repo,
        &paths,
        "msg-next",
        &HybridRecallRequest {
            requesting_orchestrator_id: "beta".to_string(),
            conversation_id: None,
            query_text: "will fail".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        &MemoryBulletinOptions {
            max_chars: 500,
            generated_at: 2,
        },
        None,
    )
    .expect("fallback bulletin");

    assert_eq!(fallback.rendered, "previous-bulletin");

    fs::remove_file(previous).expect("remove previous");
    fs::remove_file(paths.bulletins.join("msg-next.json")).expect("remove next");

    let empty = generate_bulletin_for_message(
        &repo,
        &paths,
        "msg-empty",
        &HybridRecallRequest {
            requesting_orchestrator_id: "beta".to_string(),
            conversation_id: None,
            query_text: "will fail".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        &MemoryBulletinOptions {
            max_chars: 500,
            generated_at: 3,
        },
        None,
    )
    .expect("empty fallback");

    assert!(empty.rendered.is_empty());
    let events = parse_memory_log_events(&paths.log_file);
    assert!(events.iter().any(|event| {
        event["event"] == "memory.bulletin.fallback"
            && event["fallback"] == "previous_snapshot"
            && event["message_id"] == "msg-next"
    }));
    assert!(events.iter().any(|event| {
        event["event"] == "memory.bulletin.fallback"
            && event["fallback"] == "empty_bulletin"
            && event["message_id"] == "msg-empty"
    }));
}

#[test]
fn bulletin_fallback_uses_latest_generated_at_not_filename_order() {
    let (_dir, repo, paths) = setup_repo();
    fs::create_dir_all(&paths.bulletins).expect("bulletins dir");

    fs::write(
        paths.bulletins.join("zzz-older-name.json"),
        r#"{"rendered":"older","citations":[],"sections":[],"generatedAt":10}"#,
    )
    .expect("write older bulletin");
    fs::write(
        paths.bulletins.join("aaa-newer-name.json"),
        r#"{"rendered":"newer","citations":[],"sections":[],"generatedAt":20}"#,
    )
    .expect("write newer bulletin");

    let fallback = generate_bulletin_for_message(
        &repo,
        &paths,
        "msg-fallback",
        &HybridRecallRequest {
            requesting_orchestrator_id: "beta".to_string(),
            conversation_id: None,
            query_text: "will fail".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        &MemoryBulletinOptions {
            max_chars: 500,
            generated_at: 30,
        },
        None,
    )
    .expect("fallback bulletin");

    assert_eq!(fallback.rendered, "newer");
}

#[test]
fn recall_scope_denial_logs_structured_fields() {
    let (_dir, repo, paths) = setup_repo();

    let _ = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "beta".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        None,
        &paths.log_file,
    )
    .expect_err("expected scope denial");

    let events = parse_memory_log_events(&paths.log_file);
    let denial = events
        .iter()
        .find(|event| event["event"] == "memory.recall.scope_denied")
        .expect("missing scope denial event");
    assert_eq!(denial["requested_orchestrator_id"], "beta");
    assert_eq!(denial["available_orchestrator_id"], "alpha");
}

#[test]
fn vector_adapter_contract_reads_embedding_rows() {
    let (dir, repo, _) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let node = memory_node(
        "mem-v1",
        "alpha",
        MemoryNodeType::Fact,
        "wizard vector",
        "vector",
        60,
        1.0,
        100,
        Some(canonical(&src)),
        None,
    );
    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k1", Some(canonical(&src))),
        &[node],
        &[],
    )
    .expect("persist");

    repo.upsert_embedding("mem-v1", &[0.9_f32, 0.1_f32], 100)
        .expect("upsert embedding");

    let out = query_vector(&repo, Some(vec![1.0_f32, 0.0_f32]), 10).expect("vector query");
    match out {
        VectorQueryOutcome::Ranked(items) => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].memory.memory_id, "mem-v1");
        }
        other => panic!("expected ranked vector results, got {other:?}"),
    }
}

#[test]
fn bulletin_truncation_prioritizes_goal_todo_decision_over_knowledge() {
    let recall = direclaw::memory::HybridRecallResult {
        mode: HybridRecallResultMode::FullTextOnly,
        memories: vec![
            direclaw::memory::HybridRecallMemory {
                memory: memory_node(
                    "goal-1",
                    "alpha",
                    MemoryNodeType::Goal,
                    "goal",
                    "goal",
                    100,
                    1.0,
                    1_800_000_000,
                    None,
                    None,
                ),
                provenance: direclaw::memory::MemoryProvenanceHandle {
                    source_type: MemorySourceType::Manual,
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                    captured_by: MemoryCapturedBy::System,
                },
                citation: direclaw::memory::MemoryCitation {
                    memory_id: "goal-1".to_string(),
                    source_type: "manual".to_string(),
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                },
                snippet: None,
                snippet_span_id: None,
                final_score: 1.0,
                unresolved_contradiction: false,
            },
            direclaw::memory::HybridRecallMemory {
                memory: memory_node(
                    "todo-1",
                    "alpha",
                    MemoryNodeType::Todo,
                    "todo",
                    "todo",
                    100,
                    1.0,
                    1_800_000_000,
                    None,
                    None,
                ),
                provenance: direclaw::memory::MemoryProvenanceHandle {
                    source_type: MemorySourceType::Manual,
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                    captured_by: MemoryCapturedBy::System,
                },
                citation: direclaw::memory::MemoryCitation {
                    memory_id: "todo-1".to_string(),
                    source_type: "manual".to_string(),
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                },
                snippet: None,
                snippet_span_id: None,
                final_score: 1.0,
                unresolved_contradiction: false,
            },
            direclaw::memory::HybridRecallMemory {
                memory: memory_node(
                    "decision-1",
                    "alpha",
                    MemoryNodeType::Decision,
                    "decision",
                    "decision",
                    100,
                    1.0,
                    1_800_000_000,
                    None,
                    None,
                ),
                provenance: direclaw::memory::MemoryProvenanceHandle {
                    source_type: MemorySourceType::Manual,
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                    captured_by: MemoryCapturedBy::System,
                },
                citation: direclaw::memory::MemoryCitation {
                    memory_id: "decision-1".to_string(),
                    source_type: "manual".to_string(),
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                },
                snippet: None,
                snippet_span_id: None,
                final_score: 1.0,
                unresolved_contradiction: false,
            },
            direclaw::memory::HybridRecallMemory {
                memory: memory_node(
                    "fact-1",
                    "alpha",
                    MemoryNodeType::Fact,
                    "fact",
                    "fact fact fact fact fact fact fact fact fact fact",
                    10,
                    1.0,
                    1_800_000_000,
                    None,
                    None,
                ),
                provenance: direclaw::memory::MemoryProvenanceHandle {
                    source_type: MemorySourceType::Manual,
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                    captured_by: MemoryCapturedBy::System,
                },
                citation: direclaw::memory::MemoryCitation {
                    memory_id: "fact-1".to_string(),
                    source_type: "manual".to_string(),
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                },
                snippet: None,
                snippet_span_id: None,
                final_score: 1.0,
                unresolved_contradiction: false,
            },
            direclaw::memory::HybridRecallMemory {
                memory: memory_node(
                    "fact-2",
                    "alpha",
                    MemoryNodeType::Fact,
                    "fact",
                    "fact fact fact fact fact fact fact fact fact fact",
                    10,
                    1.0,
                    1_800_000_000,
                    None,
                    None,
                ),
                provenance: direclaw::memory::MemoryProvenanceHandle {
                    source_type: MemorySourceType::Manual,
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                    captured_by: MemoryCapturedBy::System,
                },
                citation: direclaw::memory::MemoryCitation {
                    memory_id: "fact-2".to_string(),
                    source_type: "manual".to_string(),
                    source_path: None,
                    conversation_id: None,
                    workflow_run_id: None,
                    step_id: None,
                },
                snippet: None,
                snippet_span_id: None,
                final_score: 1.0,
                unresolved_contradiction: false,
            },
        ],
        edges: Vec::new(),
    };

    let bulletin = build_memory_bulletin(
        &recall,
        &MemoryBulletinOptions {
            max_chars: 180,
            generated_at: 1_800_000_100,
        },
    );

    let knowledge = bulletin
        .sections
        .iter()
        .find(|section| section.name == BulletinSectionName::KnowledgeSummary)
        .expect("knowledge section present");
    let goals = bulletin
        .sections
        .iter()
        .find(|section| section.name == BulletinSectionName::ActiveGoals)
        .expect("goal section present");
    let todos = bulletin
        .sections
        .iter()
        .find(|section| section.name == BulletinSectionName::OpenTodos)
        .expect("todo section present");
    let decisions = bulletin
        .sections
        .iter()
        .find(|section| section.name == BulletinSectionName::RecentDecisions)
        .expect("decision section present");

    assert!(knowledge.lines.is_empty());
    assert!(!goals.lines.is_empty());
    assert!(!todos.lines.is_empty());
    assert!(!decisions.lines.is_empty());
}

#[test]
fn rrf_merge_uses_expected_rank_math_from_text_and_vector_lists() {
    let (dir, repo, paths) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let now = 1_800_000_000;
    let nodes = vec![
        memory_node(
            "mem-a",
            "alpha",
            MemoryNodeType::Fact,
            "wizard alpha alpha",
            "mem-a",
            0,
            1.0,
            now,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "mem-b",
            "alpha",
            MemoryNodeType::Fact,
            "wizard alpha",
            "mem-b",
            0,
            1.0,
            now,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "mem-c",
            "alpha",
            MemoryNodeType::Fact,
            "wizard beta",
            "mem-c",
            0,
            1.0,
            now,
            Some(canonical(&src)),
            None,
        ),
    ];
    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k-rrf", Some(canonical(&src))),
        &nodes,
        &[],
    )
    .expect("persist");
    repo.upsert_embedding("mem-a", &[1.0_f32, 0.0_f32], now)
        .expect("embed a");
    repo.upsert_embedding("mem-b", &[0.8_f32, 0.2_f32], now)
        .expect("embed b");
    repo.upsert_embedding("mem-c", &[0.2_f32, 0.8_f32], now)
        .expect("embed c");

    let text_hits = query_full_text(&repo, "wizard", 10).expect("text hits");
    let vector_hits = query_vector(&repo, Some(vec![1.0_f32, 0.0_f32]), 10).expect("vector hits");
    let vector_hits = match vector_hits {
        VectorQueryOutcome::Ranked(items) => items,
        other => panic!("expected ranked vectors, got {other:?}"),
    };

    let k = 10;
    let recall = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: Some(vec![1.0_f32, 0.0_f32]),
        },
        &MemoryRecallOptions {
            top_n: 10,
            rrf_k: k,
            top_k_text: 10,
            top_k_vector: 10,
            now_unix_seconds: now,
        },
        None,
        &paths.log_file,
    )
    .expect("hybrid recall");

    for entry in &recall.memories {
        let text_rrf = text_hits
            .iter()
            .find(|hit| hit.memory.memory_id == entry.memory.memory_id)
            .map(|hit| 1.0_f64 / (k as f64 + hit.rank as f64))
            .unwrap_or(0.0);
        let vector_rrf = vector_hits
            .iter()
            .find(|hit| hit.memory.memory_id == entry.memory.memory_id)
            .map(|hit| 1.0_f64 / (k as f64 + hit.rank as f64))
            .unwrap_or(0.0);
        let expected = text_rrf + vector_rrf;
        approx_eq(entry.final_score, expected);
    }
}

#[test]
fn scoring_modifiers_importance_recency_confidence_and_contradiction_affect_order() {
    let (dir, repo, paths) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let now = 1_800_000_000;
    let ninety_days = 90 * 86_400;
    let nodes = vec![
        memory_node(
            "a",
            "alpha",
            MemoryNodeType::Fact,
            "wizard shared text",
            "a",
            0,
            1.0,
            now,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "b",
            "alpha",
            MemoryNodeType::Fact,
            "wizard shared text",
            "b",
            60,
            1.0,
            now,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "c",
            "alpha",
            MemoryNodeType::Fact,
            "wizard shared text",
            "c",
            100,
            0.5,
            now,
            Some(canonical(&src)),
            None,
        ),
        memory_node(
            "d",
            "alpha",
            MemoryNodeType::Fact,
            "wizard shared text",
            "d",
            100,
            1.0,
            now - ninety_days,
            Some(canonical(&src)),
            None,
        ),
    ];
    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k-modifiers", Some(canonical(&src))),
        &nodes,
        &[direclaw::memory::MemoryEdge {
            edge_id: "edge-b-c".to_string(),
            from_memory_id: "b".to_string(),
            to_memory_id: "c".to_string(),
            edge_type: direclaw::memory::MemoryEdgeType::Contradicts,
            weight: 0.9,
            created_at: now,
            reason: Some("conflict".to_string()),
        }],
    )
    .expect("persist");

    let recall = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions {
            top_n: 10,
            rrf_k: 1,
            top_k_text: 10,
            top_k_vector: 0,
            now_unix_seconds: now,
        },
        None,
        &paths.log_file,
    )
    .expect("recall");

    assert_eq!(
        recall
            .memories
            .iter()
            .map(|entry| entry.memory.memory_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c", "d"]
    );
    let score_a = recall
        .memories
        .iter()
        .find(|entry| entry.memory.memory_id == "a")
        .expect("a")
        .final_score;
    let score_b = recall
        .memories
        .iter()
        .find(|entry| entry.memory.memory_id == "b")
        .expect("b")
        .final_score;
    let score_c = recall
        .memories
        .iter()
        .find(|entry| entry.memory.memory_id == "c")
        .expect("c")
        .final_score;
    let score_d = recall
        .memories
        .iter()
        .find(|entry| entry.memory.memory_id == "d")
        .expect("d")
        .final_score;

    assert!(
        score_b < score_a,
        "contradiction penalty should reduce b below a"
    );
    assert!(score_c < score_b, "confidence should reduce c below b");
    assert!(score_d < score_c, "recency decay should reduce d below c");
    assert!(
        recall
            .memories
            .iter()
            .find(|entry| entry.memory.memory_id == "b")
            .expect("b contradiction")
            .unresolved_contradiction
    );
}

#[test]
fn recall_returns_error_for_invalid_edge_type_in_database() {
    let (dir, repo, paths) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");
    let node = memory_node(
        "m1",
        "alpha",
        MemoryNodeType::Fact,
        "wizard",
        "wizard",
        10,
        1.0,
        1_800_000_000,
        Some(canonical(&src)),
        None,
    );
    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k-invalid-edge", Some(canonical(&src))),
        &[node],
        &[],
    )
    .expect("persist");

    let connection = rusqlite::Connection::open(repo.database_path()).expect("open sqlite");
    connection
        .execute(
            "INSERT INTO memory_edges (
                orchestrator_id, edge_id, from_memory_id, to_memory_id, edge_type, weight, created_at, reason
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                "alpha",
                "bad-edge",
                "m1",
                "m1",
                "UnknownEdgeType",
                0.8_f32,
                1_800_000_000_i64,
                "bad"
            ],
        )
        .expect("insert bad edge");

    let err = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        None,
        &paths.log_file,
    )
    .expect_err("invalid edge type should fail");

    assert!(matches!(err, MemoryRecallError::InvalidEdgeType { .. }));
}

#[test]
fn recall_returns_error_for_invalid_numeric_shapes_in_database() {
    let (dir, repo, paths) = setup_repo();
    let src = dir.path().join("source.txt");
    fs::write(&src, "x").expect("write source");

    let node = memory_node(
        "bad-num",
        "alpha",
        MemoryNodeType::Fact,
        "wizard",
        "wizard",
        10,
        0.8,
        1_800_000_000,
        Some(canonical(&src)),
        None,
    );
    repo.upsert_nodes_and_edges(
        &source_record("alpha", "k-bad-num", Some(canonical(&src))),
        &[node],
        &[],
    )
    .expect("persist");

    let connection = rusqlite::Connection::open(repo.database_path()).expect("open sqlite");

    connection
        .execute(
            "UPDATE memories SET importance = 150 WHERE memory_id = 'bad-num'",
            [],
        )
        .expect("set invalid importance");
    let err = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        None,
        &paths.log_file,
    )
    .expect_err("invalid importance should fail");
    assert!(matches!(err, MemoryRecallError::Sql { .. }));
    assert!(err.to_string().contains("invalid importance"));

    connection
        .execute(
            "UPDATE memories SET importance = 50, confidence = 1.5 WHERE memory_id = 'bad-num'",
            [],
        )
        .expect("set invalid confidence");
    let err = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        None,
        &paths.log_file,
    )
    .expect_err("invalid confidence should fail");
    assert!(matches!(err, MemoryRecallError::Sql { .. }));
    assert!(err.to_string().contains("invalid confidence"));

    connection
        .execute(
            "INSERT INTO memory_edges (
                orchestrator_id, edge_id, from_memory_id, to_memory_id, edge_type, weight, created_at, reason
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                "alpha",
                "bad-weight",
                "bad-num",
                "bad-num",
                "RelatedTo",
                2.0_f32,
                1_800_000_000_i64,
                "bad"
            ],
        )
        .expect("insert invalid edge weight");
    connection
        .execute(
            "UPDATE memories SET confidence = 0.9 WHERE memory_id = 'bad-num'",
            [],
        )
        .expect("restore confidence");
    let err = hybrid_recall(
        &repo,
        &HybridRecallRequest {
            requesting_orchestrator_id: "alpha".to_string(),
            conversation_id: None,
            query_text: "wizard".to_string(),
            query_embedding: None,
        },
        &MemoryRecallOptions::default(),
        None,
        &paths.log_file,
    )
    .expect_err("invalid edge weight should fail");
    assert!(matches!(err, MemoryRecallError::InvalidEdgeWeight { .. }));
}

#[test]
fn bulletin_hard_caps_tiny_max_chars_deterministically() {
    let memory_id = "goal-1".to_string();
    let recall = direclaw::memory::HybridRecallResult {
        mode: HybridRecallResultMode::FullTextOnly,
        memories: vec![direclaw::memory::HybridRecallMemory {
            memory: memory_node(
                &memory_id,
                "alpha",
                MemoryNodeType::Goal,
                "goal",
                "keep this goal",
                100,
                1.0,
                1_800_000_000,
                None,
                None,
            ),
            provenance: direclaw::memory::MemoryProvenanceHandle {
                source_type: MemorySourceType::Manual,
                source_path: None,
                conversation_id: None,
                workflow_run_id: None,
                step_id: None,
                captured_by: MemoryCapturedBy::System,
            },
            citation: direclaw::memory::MemoryCitation {
                memory_id,
                source_type: "manual".to_string(),
                source_path: None,
                conversation_id: None,
                workflow_run_id: None,
                step_id: None,
            },
            snippet: None,
            snippet_span_id: None,
            final_score: 1.0,
            unresolved_contradiction: false,
        }],
        edges: Vec::new(),
    };

    let a = build_memory_bulletin(
        &recall,
        &MemoryBulletinOptions {
            max_chars: 8,
            generated_at: 1,
        },
    );
    let b = build_memory_bulletin(
        &recall,
        &MemoryBulletinOptions {
            max_chars: 8,
            generated_at: 1,
        },
    );

    assert!(a.rendered.len() <= 8);
    assert_eq!(a.rendered, b.rendered);
}
