# Phase 12: Limits, Retries, Timeouts, and Runtime Integration

## Goal
Enforce orchestration safety controls in live execution and integrate the workflow engine into queue/runtime loops with recovery-safe behavior.

## Dependencies
- `docs/build/spec/02-queue-processing.md`
- `docs/build/spec/05-workflow-orchestration.md`
- `docs/build/spec/10-daemon-operations.md`
- `docs/build/workflow-system-implementation-plan.md`

## Tasks

### P12-T01 Enforce max iterations, run timeout, step timeout, and max retries in engine loop

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Limits from orchestrator/workflow/step configuration are applied in precedence order.
  - Step retries stop exactly at configured retry ceiling.
  - Timeout and iteration-limit violations transition runs to terminal failure with explicit reason.
- Verification:
  - Add unit tests for limit precedence and boundary conditions.
  - Add integration tests for each safety limiter in live step execution.
  - Run `nix-shell --run "cargo test --test orchestrator_workflow_engine"`.

### P12-T02 Integrate engine execution path into queue processor runtime

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Queue-claimed workflow starts run engine execution, not selector-only metadata updates.
  - Workflow-bound messages (`workflow_run_id`) can resume existing runs correctly.
  - Queue failures still requeue payloads with no data loss.
- Verification:
  - Expand queue/runtime integration tests to assert step execution artifacts post-claim.
  - Add restart test for in-flight run recovery path.
  - Run `nix-shell --run "cargo test --test message_flow_queue_orchestrator_provider_e2e --test queue_lifecycle"`.

### P12-T03 Preserve per-key ordering and concurrency semantics during workflow execution

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Workflow execution does not violate ordering guarantees for same ordering key.
  - Independent keys still execute concurrently where allowed.
  - No deadlock/starvation introduced by multi-step run execution.
- Verification:
  - Add integration tests with mixed ordering keys and multi-step workflows.
  - Add stress test fixture for concurrent runs under bounded workers.
  - Run `nix-shell --run "cargo test --test queue_scheduler --test message_flow_queue_orchestrator_provider_e2e"`.

### P12-T04 Validate operational observability and failure diagnostics in runtime

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Runtime logs clearly report step failures, limit triggers, and transition outcomes.
  - `workflow status/progress` remain read-only and accurate during/after failures.
  - Failure events are inspectable from persisted artifacts without ambiguity.
- Verification:
  - Add integration assertions on runtime logs/progress snapshots for failure scenarios.
  - Run `nix-shell --run "cargo test --all"`.
  - Run `nix-shell --run "cargo clippy --all-targets --all-features -- -D warnings"`.
