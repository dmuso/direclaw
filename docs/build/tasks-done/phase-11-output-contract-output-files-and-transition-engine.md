# Phase 11: Output Contract, Output Files, and Transition Engine

## Goal
Implement strict output handling and transition rules so steps produce validated outputs, write mapped files safely, and advance correctly through workflow graphs.

## Dependencies
- `docs/build/spec/05-workflow-orchestration.md`
- `docs/build/spec/08-file-exchange.md`
- `docs/build/spec/12-reliability-compat-testing.md`
- `docs/build/workflow-system-implementation-plan.md`

## Tasks

### P11-T01 Enforce step `outputs` contract and key validation

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - When `outputs` is configured, all declared keys must exist in parsed result payload.
  - Missing required output keys fail step attempt deterministically.
  - Output validation errors are persisted with key-level detail.
- Verification:
  - Add unit tests for required/optional output key cases.
  - Add integration test for missing output key -> retry/fail behavior.
  - Run `nix-shell --run "cargo test --all"`.

### P11-T02 Materialize `output_files` mappings with strict path safety

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Output files are written under run/step/attempt scoped output root only.
  - Absolute paths and traversal (`..`) are blocked and logged.
  - Written artifacts are deterministic and replayable from attempt metadata.
- Verification:
  - Add unit tests for path interpolation and traversal denial.
  - Add integration test verifying created files and expected contents.
  - Add security integration test for malicious templates.
  - Run `nix-shell --run "cargo test --test file_semantics --test orchestrator_workflow_engine"`.

### P11-T03 Implement transition engine for `agent_task` and `agent_review`

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `agent_task` transitions via `next` or lexical fallback.
  - `agent_review` transitions via `on_approve`/`on_reject` using parsed `decision`.
  - Invalid/missing transition targets fail run with explicit error.
- Verification:
  - Add unit tests for transition resolution matrix.
  - Add integration tests for approve path, reject path, and missing target failure.
  - Run `nix-shell --run "cargo test --test orchestrator_workflow_engine"`.

### P11-T04 Persist complete attempt artifacts for outputs and transitions

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Each attempt record contains outputs and resolved next step id.
  - Progress snapshots reflect transition outcomes without ambiguity.
  - Terminal transitions persist final summary and final-state reason.
- Verification:
  - Add integration tests asserting attempt record content and progress consistency.
  - Run `nix-shell --run "cargo test --all"`.
  - Run `nix-shell --run "cargo clippy --all-targets --all-features -- -D warnings"`.
