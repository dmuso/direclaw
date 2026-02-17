# Phase 16: Memory Retrieval and Bulletin

## Goal
Deliver hybrid recall (vector + full-text + RRF), ranking controls, and deterministic per-message bulletin generation with citations.

## Tasks

### P16-T01 Implement full-text and vector retrieval adapters

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Retrieval can independently query full-text and vector indexes.
  - Missing embeddings degrade gracefully to full-text-only behavior.
  - Retrieval interfaces return typed result sets with provenance handles.
- Automated Test Requirements:
  - Unit tests for full-text query behavior.
  - Unit tests for vector adapter contracts and embedding-missing fallback.

### P16-T02 Implement Reciprocal Rank Fusion merge and scoring modifiers

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - RRF merge uses configured parameters and is deterministic.
  - Importance, recency, confidence, and contradiction modifiers are applied consistently.
  - Final result set respects `topN` limits and includes citation-ready metadata.
- Automated Test Requirements:
  - Unit tests for RRF rank math and tie-handling determinism.
  - Unit tests for scoring modifier effects on expected ordering.

### P16-T03 Enforce memory recall scope and access controls

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Recall allows cross-conversation only within same orchestrator.
  - Cross-orchestrator recall is denied and logged.
  - Source path checks honor workspace/shared access constraints.
- Automated Test Requirements:
  - Integration tests for allowed same-orchestrator recall.
  - Integration tests for denied cross-orchestrator attempts with explicit logs/errors.

### P16-T04 Implement per-message memory bulletin generation

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Bulletin is generated for each message and includes required named sections.
  - Bulletin includes contributing `memoryId` citations.
  - Bulletin truncation prioritizes Goal/Todo/Decision and is deterministic.
  - Bulletin failure falls back to prior bulletin or empty payload with warning logs.
- Automated Test Requirements:
  - Unit tests for bulletin section assembly and deterministic truncation.
  - Integration tests for fallback behavior and citation presence.
