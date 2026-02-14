# Phase 01: Runtime Supervisor and Worker Lifecycle

## Goal

Replace simulated daemon state with a real runtime supervisor that manages long-lived workers and reports actual health.

## Plan Context

Primary reference:
- `docs/build/release-readiness-plan.md`

## Tasks

### P01-T01 Implement supervisor runtime and process ownership model

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Add a supervisor entrypoint that starts/stops worker loops.
  - Persist runtime metadata (PID, start time, worker states) in a deterministic state file.
  - Prevent double-start using lock/PID checks.
  - Ensure stale PID cleanup logic is deterministic and safe.
- Acceptance Criteria:
  - `start` launches one active supervisor instance.
  - `start` fails with clear error if supervisor is already running.
  - `stop` targets the active instance and exits cleanly.
- Automated Test Requirements:
  - Integration tests for start/stop idempotency and duplicate start protection.
  - Unit tests for PID/lock state transitions.

### P01-T02 Implement worker lifecycle manager

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Define worker contract (initialize, run loop, shutdown signal, health snapshot).
  - Register workers: queue processor, orchestrator dispatcher, Slack worker (if enabled), heartbeat (if enabled).
  - Add periodic health heartbeat from each worker to supervisor state.
- Acceptance Criteria:
  - Worker states are real-time and independently tracked.
  - `status` reports true running/stopped/error states per worker.
- Automated Test Requirements:
  - Unit tests for worker registry/health transitions.
  - Integration test where one worker fails and supervisor reports degraded health.

### P01-T03 Implement graceful shutdown and restart semantics

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Implement shutdown signal propagation to worker loops.
  - Add timeout + forced termination fallback with explicit logging.
  - Implement `restart` as a full stop/start sequence preserving config context.
- Acceptance Criteria:
  - `stop` exits cleanly and leaves no orphan worker loops.
  - `restart` recovers all enabled workers and status reflects fresh runtime start.
- Automated Test Requirements:
  - Integration tests for graceful stop behavior and restart correctness.
  - Fault-injection test for slow worker shutdown.

### P01-T04 Upgrade `status` and `logs` to operationally useful output

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Ensure `status` includes per-worker state, last health timestamp, and error summary.
  - Ensure `logs` can show recent structured events from runtime log files.
  - Keep output stable enough for CLI tests and human operators.
- Acceptance Criteria:
  - Operators can identify unhealthy worker and reason from `status` + `logs` alone.
- Automated Test Requirements:
  - Integration tests with snapshot assertions for key status/log fields.

