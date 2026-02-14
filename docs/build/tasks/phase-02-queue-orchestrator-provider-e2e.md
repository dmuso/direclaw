# Phase 02: Queue, Orchestrator, and Provider End-to-End Execution

## Goal

Wire queue processing to orchestrator routing and provider execution in the production runtime path with reliability guarantees.

## Plan Context

Primary reference:
- `docs/build/release-readiness-plan.md`

## Tasks

### P02-T01 Implement continuous queue processor worker loop

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - In worker loop, claim oldest incoming queue file, move to processing, and deserialize payload.
  - Use deterministic polling interval and backoff for empty queue.
  - On success, persist outgoing and remove processing file.
  - On recoverable error, requeue safely without dropping payload.
- Acceptance Criteria:
  - Queue lifecycle follows `incoming -> processing -> outgoing` for successful messages.
  - Failures do not silently drop messages.
- Automated Test Requirements:
  - Integration tests for success path and requeue-on-failure behavior.
  - Recovery tests for malformed payload handling.

### P02-T02 Wire orchestrator routing in runtime flow

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - For each claimed message, resolve channel profile -> orchestrator.
  - Execute `process_queued_message` with real runtime dependencies.
  - Persist selector and workflow artifacts according to existing store contracts.
- Acceptance Criteria:
  - Channel-originated messages pass through orchestrator selection/routing path.
  - Routed action outcomes are persisted and observable.
- Automated Test Requirements:
  - Integration test for dispatch path with persisted selector artifacts.
  - Negative test for unknown profile/missing orchestrator failure handling.

### P02-T03 Integrate provider execution for selector and workflow actions

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Use `run_provider` in runtime orchestration path (not only unit tests).
  - Implement file-backed prompt/context artifact creation for each invocation.
  - Capture invocation logs (command form, cwd, model, exit/timeout status).
- Acceptance Criteria:
  - Runtime path performs real provider invocation for applicable actions.
  - Provider failures produce deterministic state transitions and logs.
- Automated Test Requirements:
  - Integration test with mock provider binaries validating invocation path.
  - Tests for non-zero exit, timeout, and parse-failure outcomes.

### P02-T04 Implement startup recovery for partial processing files

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - On supervisor startup, scan `queue/processing` for stale entries.
  - Requeue or reconcile each entry deterministically to prevent dead-letter loss.
  - Log recovery actions for auditability.
- Acceptance Criteria:
  - Restart after interruption does not lose or indefinitely strand queue messages.
- Automated Test Requirements:
  - Integration test simulating crash between claim and completion, then restart recovery.

### P02-T05 Enforce runtime ordering and concurrency rules

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Integrate per-key scheduling for workflow/conversation ordering.
  - Maintain concurrency across independent keys.
- Acceptance Criteria:
  - Messages for same ordering key execute sequentially.
  - Independent keys can progress concurrently.
- Automated Test Requirements:
  - Integration tests validating sequential same-key behavior and concurrent different-key behavior.

