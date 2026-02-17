use direclaw::memory::{
    compute_ingest_idempotency_key, extract_candidates_from_ingest_file,
    persist_workflow_output_memories, MemoryCapturedBy, MemoryEdge, MemoryEdgeType, MemoryNode,
    MemoryNodeType, MemoryPaths, MemoryRepository, MemorySource, MemorySourceRecord,
    MemorySourceType, MemoryStatus, WorkflowOutputWriteback,
};
use direclaw::runtime::tick_memory_worker;
use rusqlite::Connection;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn settings_yaml(workspace: &Path) -> String {
    format!(
        r#"
workspaces_path: {workspace}
shared_workspaces: {{}}
orchestrators:
  alpha:
    private_workspace: {workspace}/alpha
    shared_access: []
channel_profiles: {{}}
monitoring: {{}}
channels: {{}}
memory:
  enabled: true
  bulletin_mode: every_message
  retrieval:
    top_n: 20
    rrf_k: 60
  ingest:
    enabled: true
    max_file_size_mb: 25
  scope:
    cross_orchestrator: false
"#,
        workspace = workspace.display()
    )
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().expect("canonicalize")
}

fn sample_source(path: &Path) -> MemorySource {
    MemorySource {
        source_type: MemorySourceType::IngestFile,
        source_path: Some(canonical(path)),
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::Extractor,
    }
}

fn sample_node(orchestrator_id: &str, source_path: &Path) -> MemoryNode {
    MemoryNode {
        memory_id: "mem-1".to_string(),
        orchestrator_id: orchestrator_id.to_string(),
        node_type: MemoryNodeType::Fact,
        importance: 70,
        content: "normalized content".to_string(),
        summary: "summary".to_string(),
        confidence: 0.8,
        source: sample_source(source_path),
        status: MemoryStatus::Active,
        created_at: 1_700_000_000,
        updated_at: 1_700_000_000,
    }
}

fn sample_edge() -> MemoryEdge {
    MemoryEdge {
        edge_id: "edge-1".to_string(),
        from_memory_id: "mem-1".to_string(),
        to_memory_id: "missing".to_string(),
        edge_type: MemoryEdgeType::RelatedTo,
        weight: 0.9,
        created_at: 1_700_000_000,
        reason: Some("test".to_string()),
    }
}

#[test]
fn repository_creates_schema_tables_and_indexes() {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("memory.db");
    let repo = MemoryRepository::open(&db, "alpha").expect("open repository");
    repo.ensure_schema().expect("ensure schema");

    let table_names = repo.table_names().expect("list tables");
    assert!(table_names.contains(&"memories".to_string()));
    assert!(table_names.contains(&"memory_edges".to_string()));
    assert!(table_names.contains(&"memory_sources".to_string()));
    assert!(table_names.contains(&"memory_embeddings".to_string()));
    assert!(table_names.contains(&"memory_fts".to_string()));
}

#[test]
fn repository_rolls_back_transaction_when_edge_insert_fails() {
    let dir = tempdir().expect("tempdir");
    let source_file = dir.path().join("artifact.txt");
    fs::write(&source_file, "hello").expect("write source");

    let repo = MemoryRepository::open(&dir.path().join("memory.db"), "alpha").expect("open");
    repo.ensure_schema().expect("schema");

    let source = MemorySourceRecord {
        orchestrator_id: "alpha".to_string(),
        idempotency_key: "key-1".to_string(),
        source_type: MemorySourceType::IngestFile,
        source_path: Some(canonical(&source_file)),
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::Extractor,
    };

    let node = sample_node("alpha", &source_file);
    let edge = sample_edge();

    let err = repo
        .upsert_nodes_and_edges(&source, &[node], &[edge])
        .expect_err("foreign key should fail");
    assert!(err.to_string().to_ascii_lowercase().contains("foreign key"));
    assert_eq!(repo.count_memories().expect("count memories"), 0);
}

#[test]
fn repository_enforces_orchestrator_scope_mismatch() {
    let dir = tempdir().expect("tempdir");
    let source_file = dir.path().join("artifact.txt");
    fs::write(&source_file, "hello").expect("write source");

    let repo = MemoryRepository::open(&dir.path().join("memory.db"), "alpha").expect("open");
    repo.ensure_schema().expect("schema");

    let source = MemorySourceRecord {
        orchestrator_id: "alpha".to_string(),
        idempotency_key: "key-1".to_string(),
        source_type: MemorySourceType::IngestFile,
        source_path: Some(canonical(&source_file)),
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::Extractor,
    };

    let mut node = sample_node("alpha", &source_file);
    node.orchestrator_id = "beta".to_string();

    let err = repo
        .upsert_nodes_and_edges(&source, &[node], &[])
        .expect_err("scope mismatch");
    assert!(err.to_string().contains("orchestrator scope mismatch"));
}

#[test]
fn idempotency_key_generation_is_deterministic_for_same_artifact() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("same.md");
    fs::write(&path, "same content").expect("write source");

    let bytes = fs::read(&path).expect("read");
    let a = compute_ingest_idempotency_key(&canonical(&path), &bytes);
    let b = compute_ingest_idempotency_key(&canonical(&path), &bytes);

    assert_eq!(a, b);
}

#[test]
fn repeated_ingest_of_same_file_is_idempotent_and_provenance_is_queryable() {
    let dir = tempdir().expect("tempdir");
    let workspace = dir.path().join("workspaces");
    fs::create_dir_all(workspace.join("alpha")).expect("workspace");
    let settings: direclaw::config::Settings =
        serde_yaml::from_str(&settings_yaml(&workspace)).expect("settings");

    let runtime_root = workspace.join("alpha");
    let paths = MemoryPaths::from_runtime_root(&runtime_root);
    fs::create_dir_all(&paths.ingest).expect("ingest dir");

    let ingest_path = paths.ingest.join("artifact.txt");
    fs::write(&ingest_path, "A typed memory line").expect("write ingest");
    tick_memory_worker(&settings).expect("first tick");

    let processed_copy = paths.ingest_processed.join("artifact.txt");
    let body = fs::read_to_string(&processed_copy).expect("processed source retained");
    assert_eq!(body, "A typed memory line");

    fs::write(&ingest_path, "A typed memory line").expect("rewrite ingest");
    tick_memory_worker(&settings).expect("second tick");

    let repo = MemoryRepository::open(&paths.database, "alpha").expect("open repo");
    assert_eq!(repo.count_memories().expect("count"), 1);

    let sources = repo.list_sources().expect("list sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].source_type, MemorySourceType::IngestFile);
    assert_eq!(sources[0].captured_by, MemoryCapturedBy::Extractor);
    assert_eq!(
        sources[0].source_path.as_deref(),
        Some(
            processed_copy
                .canonicalize()
                .expect("canonical processed")
                .as_path()
        )
    );
}

#[test]
fn ingest_lifecycle_moves_supported_files_to_processed_and_unsupported_to_rejected() {
    let dir = tempdir().expect("tempdir");
    let workspace = dir.path().join("workspaces");
    fs::create_dir_all(workspace.join("alpha")).expect("workspace");
    let settings: direclaw::config::Settings =
        serde_yaml::from_str(&settings_yaml(&workspace)).expect("settings");

    let runtime_root = workspace.join("alpha");
    let paths = MemoryPaths::from_runtime_root(&runtime_root);
    fs::create_dir_all(&paths.ingest).expect("ingest dir");

    let supported = paths.ingest.join("doc.md");
    let unsupported = paths.ingest.join("doc.csv");
    fs::write(&supported, "# Title\n\nBody").expect("write md");
    fs::write(&unsupported, "a,b\n1,2").expect("write csv");

    tick_memory_worker(&settings).expect("tick");

    assert!(paths.ingest_processed.join("doc.md").is_file());
    assert!(paths.ingest_rejected.join("doc.csv").is_file());

    let rejection_manifest = paths.ingest_rejected.join("doc.csv.rejection.json");
    assert!(rejection_manifest.is_file());
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(rejection_manifest).expect("read manifest"))
            .expect("parse manifest");

    assert_eq!(value["status"], "rejected");
    assert_eq!(value["error"]["code"], "unsupported_file_type");
    assert!(value["error"]["message"].is_string());
    assert!(value["sourcePath"].is_string());
}

#[test]
fn extractor_supports_txt_md_and_json_and_normalizes_content() {
    let dir = tempdir().expect("tempdir");
    let txt = dir.path().join("sample.txt");
    let md = dir.path().join("sample.md");
    let json = dir.path().join("sample.json");

    fs::write(&txt, "  hello\n\nworld  ").expect("write txt");
    fs::write(&md, "# Title\n\n  paragraph\n").expect("write md");
    fs::write(
        &json,
        r#"{
  "memories": [{
    "memoryId": "j-1",
    "type": "Fact",
    "importance": 40,
    "content": " json   content ",
    "summary": " short ",
    "confidence": 0.9,
    "status": "active"
  }],
  "edges": []
}"#,
    )
    .expect("write json");

    let txt_out = extract_candidates_from_ingest_file(
        "alpha",
        &canonical(&txt),
        &fs::read(&txt).expect("read txt"),
    )
    .expect("extract txt");
    assert_eq!(txt_out.nodes.len(), 1);
    assert_eq!(txt_out.nodes[0].content, "hello world");

    let md_out = extract_candidates_from_ingest_file(
        "alpha",
        &canonical(&md),
        &fs::read(&md).expect("read md"),
    )
    .expect("extract md");
    assert_eq!(md_out.nodes.len(), 1);
    assert!(md_out.nodes[0].content.contains("Title"));

    let json_out = extract_candidates_from_ingest_file(
        "alpha",
        &canonical(&json),
        &fs::read(&json).expect("read json"),
    )
    .expect("extract json");
    assert_eq!(json_out.nodes.len(), 1);
    assert_eq!(json_out.nodes[0].memory_id, "j-1");
    assert_eq!(json_out.nodes[0].content, "json content");
    assert_eq!(json_out.nodes[0].summary, "short");
}

#[test]
fn ingest_rejection_manifest_uses_repository_error_code_for_persist_failures() {
    let dir = tempdir().expect("tempdir");
    let workspace = dir.path().join("workspaces");
    fs::create_dir_all(workspace.join("alpha")).expect("workspace");
    let settings: direclaw::config::Settings =
        serde_yaml::from_str(&settings_yaml(&workspace)).expect("settings");

    let runtime_root = workspace.join("alpha");
    let paths = MemoryPaths::from_runtime_root(&runtime_root);
    fs::create_dir_all(&paths.ingest).expect("ingest dir");

    let broken = paths.ingest.join("broken.json");
    fs::write(
        &broken,
        r#"{
  "memories": [{
    "memoryId": "mem-1",
    "type": "Fact",
    "importance": 55,
    "content": "persist me",
    "summary": "persist",
    "confidence": 0.9,
    "status": "active"
  }],
  "edges": [{
    "edgeId": "edge-1",
    "fromMemoryId": "mem-1",
    "toMemoryId": "missing",
    "edgeType": "RelatedTo",
    "weight": 0.8
  }]
}"#,
    )
    .expect("write broken ingest");

    tick_memory_worker(&settings).expect("tick");

    let rejection_manifest = paths.ingest_rejected.join("broken.json.rejection.json");
    assert!(rejection_manifest.is_file());
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(rejection_manifest).expect("read manifest"))
            .expect("parse manifest");
    assert_eq!(value["status"], "rejected");
    assert_eq!(value["error"]["code"], "repository_error");
}

#[test]
fn repository_memory_fts_supports_match_queries() {
    let dir = tempdir().expect("tempdir");
    let source_file = dir.path().join("artifact.txt");
    fs::write(&source_file, "hello").expect("write source");

    let repo = MemoryRepository::open(&dir.path().join("memory.db"), "alpha").expect("open");
    repo.ensure_schema().expect("schema");

    let source = MemorySourceRecord {
        orchestrator_id: "alpha".to_string(),
        idempotency_key: "fts-key-1".to_string(),
        source_type: MemorySourceType::IngestFile,
        source_path: Some(canonical(&source_file)),
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::Extractor,
    };

    let mut node = sample_node("alpha", &source_file);
    node.memory_id = "mem-fts-1".to_string();
    node.content = "spaceship banana wizard".to_string();
    node.summary = "wizard summary".to_string();

    repo.upsert_nodes_and_edges(&source, &[node], &[])
        .expect("persist");

    let db = Connection::open(dir.path().join("memory.db")).expect("open sqlite");
    let matched: String = db
        .query_row(
            "
            SELECT memory_id
            FROM memory_fts
            WHERE memory_fts MATCH 'wizard'
              AND orchestrator_id = 'alpha'
            LIMIT 1
            ",
            [],
            |row| row.get(0),
        )
        .expect("fts match query");
    assert_eq!(matched, "mem-fts-1");
}

#[test]
fn workflow_output_writeback_updates_existing_memory_for_later_attempts() {
    let dir = tempdir().expect("tempdir");
    let repo = MemoryRepository::open(&dir.path().join("memory.db"), "alpha").expect("open");
    repo.ensure_schema().expect("schema");

    let mut outputs_v1 = Map::new();
    outputs_v1.insert("decision".to_string(), Value::String("approve".to_string()));
    let output_files = BTreeMap::from_iter([(
        "decision".to_string(),
        dir.path()
            .join("decision.txt")
            .canonicalize()
            .unwrap_or_else(|_| dir.path().join("decision.txt"))
            .display()
            .to_string(),
    )]);

    persist_workflow_output_memories(
        &repo,
        &WorkflowOutputWriteback {
            orchestrator_id: "alpha",
            run_id: "run-1",
            step_id: "review",
            attempt: 1,
            conversation_id: Some("thread-1"),
            outputs: &outputs_v1,
            output_files: &output_files,
            captured_at: 100,
        },
    )
    .expect("persist first attempt");

    let mut outputs_v2 = Map::new();
    outputs_v2.insert("decision".to_string(), Value::String("reject".to_string()));
    persist_workflow_output_memories(
        &repo,
        &WorkflowOutputWriteback {
            orchestrator_id: "alpha",
            run_id: "run-1",
            step_id: "review",
            attempt: 2,
            conversation_id: Some("thread-1"),
            outputs: &outputs_v2,
            output_files: &output_files,
            captured_at: 200,
        },
    )
    .expect("persist second attempt");

    assert_eq!(repo.count_memories().expect("count memories"), 1);
    assert_eq!(repo.list_sources().expect("list sources").len(), 2);

    let db = Connection::open(dir.path().join("memory.db")).expect("open sqlite");
    let (memory_id, content, updated_at): (String, String, i64) = db
        .query_row(
            "
            SELECT memory_id, content, updated_at
            FROM memories
            WHERE orchestrator_id = 'alpha'
            LIMIT 1
            ",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read memory");
    assert_eq!(memory_id, "workflow-run-1-review-decision");
    assert_eq!(content, "reject");
    assert_eq!(updated_at, 200);
}
