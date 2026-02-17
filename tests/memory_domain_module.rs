use direclaw::memory::{
    validate_confidence, validate_edge_weight, validate_importance, MemoryCapturedBy, MemoryEdge,
    MemoryEdgeType, MemoryNode, MemoryNodeType, MemorySource, MemorySourceType, MemoryStatus,
};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
fn memory_domain_module_node_enum_serialization_round_trips() {
    let node_types = [
        MemoryNodeType::Fact,
        MemoryNodeType::Preference,
        MemoryNodeType::Decision,
        MemoryNodeType::Identity,
        MemoryNodeType::Event,
        MemoryNodeType::Observation,
        MemoryNodeType::Goal,
        MemoryNodeType::Todo,
    ];

    for node_type in node_types {
        let encoded = serde_json::to_string(&node_type).expect("serialize");
        let decoded: MemoryNodeType = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, node_type);
    }
}

#[test]
fn memory_domain_module_edge_enum_serialization_round_trips() {
    let edge_types = [
        MemoryEdgeType::RelatedTo,
        MemoryEdgeType::Updates,
        MemoryEdgeType::Contradicts,
        MemoryEdgeType::CausedBy,
        MemoryEdgeType::PartOf,
    ];

    for edge_type in edge_types {
        let encoded = serde_json::to_string(&edge_type).expect("serialize");
        let decoded: MemoryEdgeType = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, edge_type);
    }
}

#[test]
fn memory_domain_module_enforces_importance_confidence_and_weight_bounds() {
    assert!(validate_importance(100).is_ok());
    assert!(validate_importance(101).is_err());

    assert!(validate_confidence(0.0).is_ok());
    assert!(validate_confidence(1.0).is_ok());
    assert!(validate_confidence(-0.1).is_err());
    assert!(validate_confidence(1.1).is_err());

    assert!(validate_edge_weight(0.0).is_ok());
    assert!(validate_edge_weight(1.0).is_ok());
    assert!(validate_edge_weight(-0.2).is_err());
    assert!(validate_edge_weight(1.5).is_err());
}

#[test]
fn memory_domain_module_enforces_scoped_source_fields() {
    let workflow_source = MemorySource {
        source_type: MemorySourceType::WorkflowOutput,
        source_path: None,
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::Extractor,
    };
    assert!(workflow_source.validate().is_err());

    let transcript_source = MemorySource {
        source_type: MemorySourceType::ChannelTranscript,
        source_path: None,
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::System,
    };
    assert!(transcript_source.validate().is_err());

    let ingest_source = MemorySource {
        source_type: MemorySourceType::IngestFile,
        source_path: Some(PathBuf::from("relative/path.md")),
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::User,
    };
    assert!(ingest_source.validate().is_err());
}

#[test]
fn memory_domain_module_requires_ingest_source_path_to_be_canonical() {
    let root = tempdir().expect("tempdir");
    let ingest_file = root.path().join("ingest.txt");
    fs::write(&ingest_file, "hello").expect("write ingest file");
    fs::create_dir_all(root.path().join("nested")).expect("nested dir");

    let canonical = ingest_file
        .canonicalize()
        .expect("canonicalize ingest file");
    let valid = MemorySource {
        source_type: MemorySourceType::IngestFile,
        source_path: Some(canonical.clone()),
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::User,
    };
    valid.validate().expect("canonical path should validate");

    let non_canonical = MemorySource {
        source_type: MemorySourceType::IngestFile,
        source_path: Some(
            canonical
                .parent()
                .expect("parent")
                .join("nested/../ingest.txt"),
        ),
        conversation_id: None,
        workflow_run_id: None,
        step_id: None,
        captured_by: MemoryCapturedBy::User,
    };
    assert!(non_canonical.validate().is_err());
}

#[test]
fn memory_domain_module_requires_workflow_output_source_path_to_be_canonical_when_present() {
    let root = tempdir().expect("tempdir");
    let output_file = root.path().join("out.txt");
    fs::write(&output_file, "hello").expect("write output file");
    fs::create_dir_all(root.path().join("nested")).expect("nested dir");

    let canonical = output_file.canonicalize().expect("canonical output file");
    let valid = MemorySource {
        source_type: MemorySourceType::WorkflowOutput,
        source_path: Some(canonical.clone()),
        conversation_id: None,
        workflow_run_id: Some("run-1".to_string()),
        step_id: Some("step-1".to_string()),
        captured_by: MemoryCapturedBy::System,
    };
    valid.validate().expect("canonical path should validate");

    let non_canonical = MemorySource {
        source_type: MemorySourceType::WorkflowOutput,
        source_path: Some(
            canonical
                .parent()
                .expect("parent")
                .join("nested/../out.txt"),
        ),
        conversation_id: None,
        workflow_run_id: Some("run-1".to_string()),
        step_id: Some("step-1".to_string()),
        captured_by: MemoryCapturedBy::System,
    };
    assert!(non_canonical.validate().is_err());
}

#[test]
fn memory_domain_module_validates_memory_node_required_fields_and_bounds() {
    let node = MemoryNode {
        memory_id: "mem-1".to_string(),
        orchestrator_id: "orch-a".to_string(),
        node_type: MemoryNodeType::Decision,
        importance: 101,
        content: "Decision text".to_string(),
        summary: "Decision summary".to_string(),
        confidence: 0.9,
        source: MemorySource {
            source_type: MemorySourceType::WorkflowOutput,
            source_path: Some(PathBuf::from("/tmp/output.json")),
            conversation_id: None,
            workflow_run_id: Some("run-1".to_string()),
            step_id: Some("step-a".to_string()),
            captured_by: MemoryCapturedBy::Extractor,
        },
        status: MemoryStatus::Active,
        created_at: 1_700_000_000,
        updated_at: 1_700_000_001,
    };

    assert!(node.validate().is_err());
}

#[test]
fn memory_domain_module_serializes_struct_fields_using_spec_camel_case() {
    let source = MemorySource {
        source_type: MemorySourceType::WorkflowOutput,
        source_path: Some(PathBuf::from("/tmp/output.json")),
        conversation_id: Some("c-1".to_string()),
        workflow_run_id: Some("run-1".to_string()),
        step_id: Some("step-1".to_string()),
        captured_by: MemoryCapturedBy::Extractor,
    };
    let node = MemoryNode {
        memory_id: "mem-1".to_string(),
        orchestrator_id: "orch-1".to_string(),
        node_type: MemoryNodeType::Fact,
        importance: 50,
        content: "content".to_string(),
        summary: "summary".to_string(),
        confidence: 0.5,
        source: source.clone(),
        status: MemoryStatus::Active,
        created_at: 100,
        updated_at: 101,
    };
    let edge = MemoryEdge {
        edge_id: "edge-1".to_string(),
        from_memory_id: "mem-1".to_string(),
        to_memory_id: "mem-2".to_string(),
        edge_type: MemoryEdgeType::RelatedTo,
        weight: 0.7,
        created_at: 100,
        reason: Some("matches".to_string()),
    };

    let source_json: Value = serde_json::to_value(&source).expect("serialize source");
    assert!(source_json.get("sourceType").is_some());
    assert!(source_json.get("sourcePath").is_some());
    assert!(source_json.get("conversationId").is_some());
    assert!(source_json.get("workflowRunId").is_some());
    assert!(source_json.get("stepId").is_some());
    assert!(source_json.get("capturedBy").is_some());

    let node_json: Value = serde_json::to_value(&node).expect("serialize node");
    assert!(node_json.get("memoryId").is_some());
    assert!(node_json.get("orchestratorId").is_some());
    assert!(node_json.get("createdAt").is_some());
    assert!(node_json.get("updatedAt").is_some());
    assert!(node_json.get("type").is_some());

    let edge_json: Value = serde_json::to_value(&edge).expect("serialize edge");
    assert!(edge_json.get("edgeId").is_some());
    assert!(edge_json.get("fromMemoryId").is_some());
    assert!(edge_json.get("toMemoryId").is_some());
    assert!(edge_json.get("edgeType").is_some());
    assert!(edge_json.get("createdAt").is_some());
}
