# Phase 12 Review: Limits, Retries, Timeouts, and Runtime Integration

## Findings Requiring Action

1. **High: Run timeout is not enforced against real elapsed runtime**
- **Spec/plan mismatch**: The implementation plan requires run timeout enforcement in live execution (`Retry, Timeout, and Limits`, `Phase 4`, and `Target Functional Contract #7`).
- **What is happening**: `WorkflowEngine::start`/`resume` seed a synthetic timestamp (`now + 1`) and `run_until_non_running` advances it by fixed increments (`+2`) per loop, regardless of real provider execution time.
- **Impact**: Long-running runs can exceed configured `run_timeout_seconds` in wall-clock time without triggering `RunTimeout`, especially when steps are slow but under per-step timeout.
- **Code refs**:
  - `src/orchestrator.rs:1029`
  - `src/orchestrator.rs:1049`
  - `src/orchestrator.rs:1078`
  - `src/orchestrator.rs:2244`
- **Action**:
  - Use real timestamps (`now_secs()` / `SystemTime`) for each safety check iteration.
  - Enforce run-timeout both before and after attempt execution using actual elapsed seconds from `run.started_at`.
  - Add an integration test where provider calls consume real time across multiple steps and verify run timeout fails deterministically.

2. **Medium: Unknown workflow-bound run IDs now requeue indefinitely instead of failing gracefully**
- **Spec/plan mismatch**: Queue/runtime integration should preserve recovery-safe behavior and avoid pathological retry loops.
- **What changed**: Workflow-bound non-status messages now call `engine.resume(run_id, now)` directly. If `run_id` is missing/invalid, this returns an error that bubbles to runtime, which requeues the same payload.
- **Impact**: Deterministic bad payloads (stale or invalid `workflow_run_id`) can livelock the queue via perpetual requeue.
- **Code refs**:
  - `src/orchestrator.rs:4223`
  - `src/runtime.rs:1156`
- **Action**:
  - Convert missing-run resume failures into a non-fatal `WorkflowStatus`/diagnostic response (similar to status path handling), or quarantine/dead-letter after bounded retries.
  - Add an E2E test for workflow-bound non-status messages with nonexistent `workflow_run_id` asserting no infinite requeue behavior.

## Verification Notes
- Executed in `nix-shell`:
  - `cargo test --test orchestrator_workflow_engine`
  - `cargo test --test message_flow_queue_orchestrator_provider_e2e`
  - `cargo test --test queue_scheduler`
  - `cargo test --all`
  - `cargo clippy --all-targets --all-features -- -D warnings`
- All commands passed, so these are behavioral/design gaps not compile/test failures in the current suite.
