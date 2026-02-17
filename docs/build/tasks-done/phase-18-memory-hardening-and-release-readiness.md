# Phase 18: Memory Hardening and Release Readiness

## Goal
Harden reliability and operational behavior, expand automated coverage, and complete release-quality acceptance for the memory subsystem.

## Tasks

### P18-T01 Implement recovery behavior and corrupted-store handling

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Corrupt `memory.db` yields explicit failure state for memory worker.
  - Orchestrator continues in degraded mode with clear status/log signal.
  - Restart behavior avoids data duplication and preserves ingest ordering guarantees.
- Automated Test Requirements:
  - Integration tests with injected DB corruption and degraded-mode assertions.
  - Integration tests for restart/replay idempotency behavior.

### P18-T02 Add observability and operational diagnostics for memory subsystem

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Structured logs capture ingest outcomes, recall scope denials, and bulletin fallback events.
  - Runtime diagnostics can report memory worker status and recent failures.
  - Log fields are stable and parseable for troubleshooting automation.
- Automated Test Requirements:
  - Unit tests for structured log field emission.
  - Integration tests asserting operational status and log events under failure scenarios.

### P18-T03 Expand end-to-end and regression coverage for memory-enabled workflows

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - E2E scenario validates memory creation, recall, and downstream workflow impact.
  - E2E scenario validates same-orchestrator cross-channel recall behavior.
  - Regression suite confirms channel -> orchestrator -> workflow path remains intact with memory enabled.
- Automated Test Requirements:
  - End-to-end tests covering creation/recall/decision carryover.
  - Regression tests for existing queue lifecycle and selector routing invariants.

### P18-T04 Pass repo quality gates in required environment and close phase

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all` pass in `nix-shell`.
  - All tasks in phases 14-18 are marked `complete` only after acceptance and tests pass.
  - Any scope adjustments are documented via appended scope notes in affected phase docs.
- Automated Test Requirements:
  - CI or local execution artifacts demonstrate full quality gate pass.
  - Checklist audit confirms no task marked complete without its required tests.

## Scope Notes
- 2026-02-17: Added explicit `memory_db_corrupt` degraded-state detection for memory worker, structured JSON memory diagnostics events (ingest outcomes, recall scope denials, bulletin fallbacks), restart/replay idempotency and ordering coverage, and memory-enabled same-orchestrator cross-channel E2E recall coverage.
- 2026-02-17: Verified release gates in required environment with `nix-shell`: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all`.
