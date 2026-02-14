# Phase 09: Workflow Engine Skeleton and Run Model

## Goal
Implement the executable workflow engine skeleton and complete run data model so runs can be started, resumed, and tracked with persisted inputs and deterministic step pointers.

## Dependencies
- `docs/build/spec/05-workflow-orchestration.md`
- `docs/build/spec/03-agent-routing-execution.md`
- `docs/build/spec/12-reliability-compat-testing.md`
- `docs/build/workflow-system-implementation-plan.md`

## Tasks

### P09-T01 Extend workflow run persistence to include workflow inputs

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Workflow run records persist input payloads as structured key/value data.
  - Existing run metadata remains backward-compatible for read operations.
  - `workflow status` and `workflow progress` expose persisted input presence/shape where appropriate.
- Verification:
  - Add unit tests for run record serialization/deserialization with inputs.
  - Add integration test ensuring `workflow run --input ...` persists values, not just input count.
  - Run `nix-shell --run "cargo test --all"`.

### P09-T02 Implement workflow engine entrypoints (`start`, `resume`, `execute-next`)

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Engine supports explicit start/resume APIs for a run.
  - Engine resolves the correct next step deterministically from run state.
  - Engine writes progress updates before and after step attempt boundaries.
- Verification:
  - Add unit tests covering next-step resolution for fresh and resumed runs.
  - Add integration test proving resume after persisted in-progress state.
  - Run `nix-shell --run "cargo test --test orchestrator_workflow_engine"`.

### P09-T03 Wire run creation path to engine start, not metadata-only transitions

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `workflow_start` triggers executable workflow progression path.
  - `workflow run` command produces runs that can execute steps without manual intervention.
  - No placeholder-only `running` state without execution attempt.
- Verification:
  - Add integration test asserting first step attempt artifact appears after run start.
  - Add negative test that start failure transitions run to explicit failed state with reason.
  - Run `nix-shell --run "cargo test --all"`.

### P09-T04 Add run/step observability baseline for engine actions

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Engine logs include run id, step id, attempt number, and transition decisions.
  - Progress snapshots clearly identify current step and expected next action.
  - Failures are persisted with actionable error strings.
- Verification:
  - Add integration assertions on persisted logs/progress fields.
  - Run `nix-shell --run "cargo test --all"`.
  - Run `nix-shell --run "cargo clippy --all-targets --all-features -- -D warnings"`.

