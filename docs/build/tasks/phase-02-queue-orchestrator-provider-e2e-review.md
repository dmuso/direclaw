# Phase 02 Review: Queue, Orchestrator, and Provider End-to-End Execution

## Scope Reviewed
- Uncommitted changes in:
  - `src/runtime.rs`
  - `src/queue.rs`
  - `src/orchestrator.rs`
  - `tests/orchestrator_workflow_engine.rs`
  - `tests/phase02_queue_orchestrator_provider_e2e.rs`
- Requirements baseline:
  - `docs/build/release-readiness-plan.md` (Phase 02 + test strategy expectations)
  - `docs/build/tasks/phase-02-queue-orchestrator-provider-e2e.md`

## Findings Requiring Action

1. **Critical: queue worker shutdown can hang indefinitely due to dropped completion accounting.**
- Evidence:
  - In stop mode, the queue loop consumes a completion with `recv_timeout` but discards it without decrementing `in_flight` or calling `scheduler.complete` (`src/runtime.rs:1038`).
  - Exit condition requires `in_flight == 0` (`src/runtime.rs:1029`), so dropped completions can prevent the worker from ever reaching zero.
  - This risks supervisor shutdown deadlock because worker threads are joined after the timeout window (`src/runtime.rs:609`).
- Why this blocks Phase 02:
  - Violates release-plan requirement to enforce reliability/no-stranding behavior in the production queue path (`docs/build/release-readiness-plan.md:103`).
- Action:
  - Handle stop-path completions identically to normal-path completions (decrement `in_flight`, `scheduler.complete`, emit event).
  - Add a regression test that requests stop while queue tasks are in flight and asserts clean worker termination.

2. **Medium: required provider timeout-path coverage is missing.**
- Evidence:
  - Phase-02 task explicitly requires tests for non-zero exit, timeout, and parse-failure outcomes (`docs/build/tasks/phase-02-queue-orchestrator-provider-e2e.md:55`).
  - Current test only covers non-zero exit and parse failure (`tests/phase02_queue_orchestrator_provider_e2e.rs:205`).
- Why this blocks Phase 02:
  - Leaves one required failure mode unverified for release-plan reliability goals (`docs/build/release-readiness-plan.md:137`).
- Action:
  - Add an integration test with a mock provider that exceeds timeout and assert deterministic fallback plus persisted invocation log fields (`status=failed`, `timedOut=true`).

3. **Medium: malformed-queue-payload and restart-recovery integration coverage is still incomplete.**
- Evidence:
  - P02-T01 requires malformed payload recovery coverage (`docs/build/tasks/phase-02-queue-orchestrator-provider-e2e.md:27`), but no test currently writes malformed queue JSON.
  - P02-T04 requires a crash-then-restart integration recovery test (`docs/build/tasks/phase-02-queue-orchestrator-provider-e2e.md:67`), while current coverage calls recovery helper directly (`tests/phase02_queue_orchestrator_provider_e2e.rs:256`) without exercising supervisor restart flow.
- Why this blocks Phase 02:
  - The release plan calls for recovery-path verification under realistic runtime lifecycle conditions (`docs/build/release-readiness-plan.md:142`).
- Action:
  - Add malformed payload test in the queue worker path asserting message is not silently dropped.
  - Add an integration test that simulates interruption after claim (message in `queue/processing`), then starts runtime and verifies automatic recovery and eventual processing.

## Validation Run
- `nix-shell --run 'cargo test --test phase02_queue_orchestrator_provider_e2e --test orchestrator_workflow_engine'` -> **passed**.
