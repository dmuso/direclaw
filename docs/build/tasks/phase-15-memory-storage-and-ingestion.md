# Phase 15: Memory Storage and Ingestion

## Goal
Implement canonical SQLite-backed persistence, ingestion pipeline lifecycle, and auditability semantics for typed memory nodes and graph edges.

## Tasks

### P15-T01 Implement SQLite schema and transactional repository layer

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `memory.db` is created per orchestrator with required tables/indexes.
  - Node and edge upserts execute transactionally.
  - Persistence enforces orchestrator scoping and explicit error surfaces.
- Automated Test Requirements:
  - Unit tests for repository CRUD/upsert behavior.
  - Integration tests validating schema creation and transaction rollback semantics.

### P15-T02 Implement memory idempotency and provenance persistence

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Ingestion/persistence uses deterministic idempotency keys for source artifacts.
  - Duplicate source processing does not create duplicate memory records.
  - Provenance fields are stored and queryable for audit trails.
- Automated Test Requirements:
  - Unit tests for idempotency key generation and duplicate detection.
  - Integration tests for repeated ingest of same file producing stable results.

### P15-T03 Build ingest file discovery and lifecycle transitions

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Worker discovers supported files in `ingest/` and routes outcomes to `processed/` or `rejected/`.
  - Rejected artifacts include machine-readable error manifests.
  - Unsupported file types are rejected deterministically with explicit reasons.
- Automated Test Requirements:
  - Integration tests for `ingest -> processed` and `ingest -> rejected` transitions.
  - Regression tests for deterministic rejection reason shape.

### P15-T04 Implement typed extraction from `.txt`, `.md`, and `.json`

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Extractor emits validated typed memory candidates and optional edges.
  - Extraction failures do not crash the worker and are logged per artifact.
  - Extracted content is normalized before persistence.
- Automated Test Requirements:
  - Unit tests for parser/extractor behavior per file type.
  - Integration tests for end-to-end ingest of representative sample files.
