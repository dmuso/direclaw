# Phase 15 Review: Memory Storage and Ingestion

Reviewed uncommitted changes against `docs/build/spec/14-memory.md`.

## Findings (Needs Action)

### 1. High: Repository failures are mislabeled as `invalid_json` in rejection manifests
- Location: `src/memory/ingest.rs:261`, `src/memory/ingest.rs:266`, `src/memory/ingest.rs:295`
- Problem:
  - Any repository failure during persistence (for example: schema/constraint/validation/sqlite failures) is wrapped as `MemoryExtractionError::InvalidJson` before writing the rejection manifest.
  - This produces an incorrect machine-readable `error.code` (`invalid_json`) even when JSON parsing succeeded and failure happened later.
- Spec impact:
  - Violates ingestion requirement that rejections include accurate machine-readable reasons.
- Action:
  - Introduce a distinct ingest/persistence rejection error classification (for example `repository_error` or finer-grained codes).
  - Update rejection manifest mapping so repository failures do not collapse into `invalid_json`.
  - Add regression tests asserting rejection code correctness for non-JSON repository failures.

### 2. Medium: `memory_fts` is created as a normal table, not an FTS-backed structure
- Location: `src/memory/repository.rs:145`
- Problem:
  - `memory_fts` is currently created with `CREATE TABLE` rather than an FTS virtual table.
  - This blocks direct full-text query capability expected by hybrid retrieval (`vector + FTS + RRF`).
- Spec impact:
  - Risks non-compliance with the retrieval contract in `docs/build/spec/14-memory.md` that requires full-text retrieval as part of hybrid recall.
- Action:
  - Replace `memory_fts` with an SQLite FTS virtual table (FTS5) and ensure upsert paths keep it synchronized with `memories`.
  - Add tests that execute actual FTS queries (not only table existence checks).

## Verification Run
- `nix-shell --run 'cargo test --all'` passed.
- `nix-shell --run 'cargo clippy --all-targets --all-features -- -D warnings'` passed.
- `nix-shell --run 'cargo fmt --all -- --check'` passed.
