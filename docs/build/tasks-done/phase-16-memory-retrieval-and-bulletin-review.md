# Phase 16 Review: Memory Retrieval and Bulletin

Reviewed uncommitted changes against `docs/build/spec/14-memory.md` and `docs/build/tasks/phase-16-memory-retrieval-and-bulletin.md`.

## Findings Requiring Action

1. High: Unauthorized source-path scope denials are not logged.
- Spec/task impact:
  - `docs/build/spec/14-memory.md:159` requires unauthorized recall scope attempts to fail closed and be logged.
  - P16-T03 requires denied attempts with explicit logs/errors (`docs/build/tasks/phase-16-memory-retrieval-and-bulletin.md:39`).
- Evidence:
  - Cross-orchestrator denial is logged, but source-path denial returns `MemoryRecallError::SourcePathAccessDenied` without a log write in `src/memory/retrieval.rs:370` and `src/memory/retrieval.rs:373`.
- Required action:
  - Append an explicit memory log entry before returning `SourcePathAccessDenied`, and add a regression test asserting the log line is present.

2. High: Bulletin size limits are not strictly enforced for small `max_chars` values.
- Spec/task impact:
  - Bulletin must enforce deterministic size limits (`docs/build/spec/14-memory.md:179`).
- Evidence:
  - `truncate_sections` stops when no section lines remain, even if rendered output still exceeds `max_chars` due to fixed section headers/placeholders (`src/memory/bulletin.rs:221`, `src/memory/bulletin.rs:238`, `src/memory/bulletin.rs:268`).
- Required action:
  - Add a final deterministic hard-cap strategy when headers-only output exceeds limit (for example, deterministic section/header truncation policy), with unit coverage for very small limits.

3. Medium: Invalid `edge_type` values are silently coerced to `RelatedTo` during recall.
- Spec/task impact:
  - Invalid enum values should fail explicitly rather than being silently accepted (`docs/build/spec/14-memory.md`, Storage Contract rules).
- Evidence:
  - Unknown edge types default to `MemoryEdgeType::RelatedTo` in `src/memory/retrieval.rs:521`.
- Required action:
  - Return an explicit typed error for unknown edge values instead of coercing, and add a test for malformed DB row behavior.

4. Medium: P16 automated test requirements for RRF/scoring/truncation priority are under-specified in current tests.
- Spec/task impact:
  - P16-T02 requires tests for RRF rank math, tie-handling determinism, and scoring modifier ordering (`docs/build/tasks/phase-16-memory-retrieval-and-bulletin.md:27`, `docs/build/tasks/phase-16-memory-retrieval-and-bulletin.md:28`).
  - P16-T04 requires deterministic truncation prioritization coverage (`docs/build/tasks/phase-16-memory-retrieval-and-bulletin.md:47`, `docs/build/tasks/phase-16-memory-retrieval-and-bulletin.md:50`).
- Evidence:
  - `rrf_merge_and_scoring_modifiers_are_deterministic` runs against an empty repo and only compares two empty result orders, so it does not verify RRF math or modifier effects (`tests/memory_retrieval_bulletin.rs:185`).
  - Bulletin test checks section/citation presence and equality across two runs, but does not assert truncation preserves Goal/Todo/Decision over lower-priority sections (`tests/memory_retrieval_bulletin.rs:495`).
- Required action:
  - Add fixture-based tests with overlapping text/vector ranks to assert RRF and tie ordering numerically.
  - Add tests that explicitly assert importance/recency/confidence/contradiction modifier impact on ordering.
  - Add truncation tests that force removals and assert Goal/Todo/Decision survive longer than lower-priority sections.
