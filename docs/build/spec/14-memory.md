# Memory Subsystem

## Scope

Defines the typed memory model, graph relationships, storage contract, ingestion pipeline, retrieval behavior, bulletin injection, access controls, reliability rules, and required tests.

This spec is additive to existing orchestrator behavior and does not change the requirement that channel-originated execution routes through the orchestrator.

## Core Model

Memory is represented as typed graph nodes plus typed edges.

### Memory Node

Required fields:

- `memoryId` (stable id)
- `orchestratorId`
- `type` (`Fact`|`Preference`|`Decision`|`Identity`|`Event`|`Observation`|`Goal`|`Todo`)
- `importance` (integer `0..100`)
- `content` (normalized plain text)
- `summary` (short plain text)
- `confidence` (float `0.0..1.0`)
- `source` (provenance object)
- `status` (`active`|`superseded`|`retracted`)
- `createdAt`
- `updatedAt`

### Memory Edge

Required fields:

- `edgeId`
- `fromMemoryId`
- `toMemoryId`
- `edgeType` (`RelatedTo`|`Updates`|`Contradicts`|`CausedBy`|`PartOf`)
- `weight` (`0.0..1.0`)
- `createdAt`

Optional fields:

- `reason` (short plain text)

## Runtime Filesystem Layout

Per orchestrator, under `<orchestrator_runtime_root>/memory`:

- `memory.db` (canonical SQLite store)
- `ingest/` (drop-zone for source files)
- `ingest/processed/` (success manifests)
- `ingest/rejected/` (failed files and error manifests)
- `bulletins/` (materialized bulletin snapshots)
- `logs/memory.log`

## Storage Contract

Canonical backend in v1:

- Per-orchestrator SQLite database at `<orchestrator_runtime_root>/memory/memory.db`

Required logical tables/indexes:

- `memories`
- `memory_edges`
- `memory_sources`
- `memory_embeddings`
- `memory_fts`

Rules:

- Node and edge writes must be transactional.
- Invalid enum values or shape violations must fail with explicit errors.
- Memory persistence is scoped by `orchestratorId` and must not leak across orchestrators.

## Provenance and Auditability

`source` must include:

- `sourceType` (`workflow_output`|`channel_transcript`|`ingest_file`|`diagnostics`|`manual`)
- `sourcePath` (absolute canonical path when file-backed)
- `conversationId` (when applicable)
- `workflowRunId` (when applicable)
- `stepId` (when applicable)
- `capturedBy` (`extractor`|`user`|`system`)

Rules:

- Contradictions must not delete prior memories; they must be represented with `Contradicts` edges.
- Supersession/retraction must be explicit via `status` and logged.

## Ingestion

Source drop path:

- `<orchestrator_runtime_root>/memory/ingest/`

Supported file inputs in v1:

- `.txt`
- `.md`
- `.json`

Pipeline:

1. Discover ingest files.
2. Parse and extract candidate typed memories.
3. Validate type, bounds, provenance, and scope.
4. Upsert nodes and edges transactionally.
5. Write success or rejection manifest.
6. Move source file to `processed/` or `rejected/`.

Rules:

- Rejections must include machine-readable reasons.
- Ingestion must be idempotent for repeated source files.
- No migration/backward compatibility promises for legacy systems are required in beta.

## Retrieval: Hybrid Recall

Hybrid ranking contract:

1. Vector similarity retrieval.
2. Full-text retrieval.
3. Merge candidate rankings via Reciprocal Rank Fusion (RRF).

Default retrieval parameters:

- `topKVector=50`
- `topKText=50`
- `rrfK=60`
- `topN=20` pre-compression

Ranking modifiers:

- importance boost
- recency decay
- confidence weighting
- contradiction penalty when unresolved

Returned bundle must include:

- selected memory nodes
- relevant graph edges
- provenance references

## Scope and Access Controls

Default cross-conversation recall policy:

- Allowed across conversations/channels only when both map to the same orchestrator.

Disallowed in v1:

- Cross-orchestrator recall.

Rules:

- Memory source path resolution must obey workspace isolation and shared-workspace allowlists.
- Unauthorized recall scope attempts must fail closed and be logged.

## Memory Bulletin

Bulletin generation policy in v1:

- Generate on every message before selector/step context assembly.

Bulletin sections:

- `knowledge_summary`
- `active_goals`
- `open_todos`
- `recent_decisions`
- `preference_profile`
- `conflicts_and_uncertainties`

Rules:

- Bulletin must cite contributing `memoryId` values.
- Bulletin must enforce deterministic size limits.
- Truncation priority must favor `Goal`, `Todo`, and `Decision` when limits are reached.
- Bulletin generation failure must not block workflow execution; runtime should fall back to last successful bulletin or an empty bulletin with warning logs.

## Orchestrator Integration

Memory retrieval must be invoked during orchestrator context assembly for:

- selector prompts
- workflow step prompts
- diagnostics investigations

Memory write-back triggers:

- parsed workflow outputs
- inbound transcript capture
- diagnostics findings

Channel-originated execution flow remains unchanged:

- channel -> queue -> orchestrator selection -> workflow dispatch

## Configuration Additions

Typed config additions (global or per-orchestrator as defined by runtime config model):

- `memory.enabled` (bool, default `true`)
- `memory.bulletin_mode` (`every_message`)
- `memory.retrieval.top_n`
- `memory.retrieval.rrf_k`
- `memory.ingest.enabled`
- `memory.ingest.max_file_size_mb`
- `memory.scope.cross_orchestrator` (must be `false` in v1)

Validation rules:

- Unknown or invalid memory config keys/types must fail validation.
- `memory.scope.cross_orchestrator=true` must fail validation in v1.

## Failure Modes and Recovery

Required behavior:

- Corrupt `memory.db`: memory worker fails fast with explicit error; orchestrator continues with memory disabled status.
- Ingest parse/validation failure: source moved to rejected with reason manifest.
- Embedding failure: memory node persists without embedding; retrieval falls back to FTS path while retries continue.
- Restart/replay: ingestion idempotency prevents duplicate memory creation from same source artifact.

## Testing Requirements

### Unit Tests

- memory type enum validation (all 8)
- edge type enum validation (all 5)
- importance/confidence bounds validation
- provenance/source path validation and canonicalization
- deterministic RRF merge behavior
- deterministic bulletin truncation/prioritization

### Integration Tests

- ingest folder lifecycle (`ingest -> processed|rejected`)
- typed node + edge persistence in SQLite
- hybrid retrieval returns merged results with citations
- contradiction handling creates graph edges and status transitions
- same-orchestrator cross-channel recall allowed
- cross-orchestrator recall denied and logged
- bulletin injected into selector/step context every message

### End-to-End Smoke Tests

- message A creates memory; message B in same orchestrator recalls it
- workflow step output produces `Decision`/`Todo` memory and affects subsequent context
- diagnostics request retrieves bounded evidence from memory graph with provenance

## Acceptance Criteria

- Typed memory schema and graph edges are strictly enforced.
- Per-orchestrator SQLite storage is canonical and transactional.
- Hybrid retrieval (vector + FTS + RRF) is implemented and test-covered.
- Bulletin is generated every message, bounded, cited, and deterministic.
- Cross-orchestrator recall is blocked by default in v1.
- Ingestion lifecycle is auditable with explicit success/rejection artifacts.
- Reliability and validation posture matches beta strictness and existing runtime guarantees.
