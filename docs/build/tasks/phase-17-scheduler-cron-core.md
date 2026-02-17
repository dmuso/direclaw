# Phase 17: Scheduler and Cron Core

## Goal
Add orchestrator-scoped scheduled automation with natural-language command routing, persistent jobs, cron/interval/once evaluation, and queue-based trigger dispatch. See `docs/build/spec/15-scheduled-automation.md` for spec.

## Tasks

### P17-T01 Add scheduler command surface and selector function exposure

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Command/function registry includes `schedule.create`, `schedule.list`, `schedule.show`, `schedule.update`, `schedule.pause`, `schedule.resume`, `schedule.delete`, and `schedule.run_now`.
  - Selector `command_invoke` validation supports scheduler function ids and typed argument schemas.
  - CLI invocation parity exists for scheduler lifecycle operations with deterministic usage/errors.
  - Unauthorized/invalid function ids are rejected with explicit selector validation errors.
- Automated Test Requirements:
  - Unit tests for scheduler function argument validation (required fields, type mismatches, unknown args).
  - Unit tests for function registry exposure through selector `availableFunctions`.
  - Integration tests for end-to-end command invocation path (`selector -> command_invoke -> schedule.*`).

### P17-T02 Implement scheduler job model, persistence, and validation

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Job persistence exists under `<orchestrator_runtime_root>/automation/jobs` with stable ids and full required metadata.
  - Schedule types `once`, `interval`, and `cron` are validated at create/update time.
  - Cron parser supports v1 5-field expressions and timezone validation for IANA ids.
  - Job state transitions (`enabled|paused|disabled|deleted`) are enforced with explicit validation errors on invalid transitions.
- Automated Test Requirements:
  - Unit tests for job schema serialization/deserialization and validation rules.
  - Unit tests for cron parser and timezone validation (valid and invalid cases).
  - Integration tests for create/update/pause/resume/delete persistence lifecycle.

### P17-T03 Implement scheduler worker trigger evaluation and misfire recovery

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Scheduler worker evaluates due jobs at least once per minute and computes deterministic `nextRunAt`.
  - Restart recovery applies configured misfire policy (`fire_once_on_recovery|skip_missed`).
  - Scheduler prevents overlapping triggers for the same job unless explicitly configured.
  - Run history is persisted for each trigger attempt under `<orchestrator_runtime_root>/automation/runs`.
- Automated Test Requirements:
  - Unit tests for next-run computation across `once`, `interval`, and `cron`.
  - Integration tests for misfire behavior after simulated downtime.
  - Regression tests for overlap protection and duplicate trigger suppression using execution ids.

### P17-T04 Dispatch scheduled triggers through queue and orchestrator routing path

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Scheduler-triggered executions are enqueued into orchestrator incoming queue, not executed by direct provider calls.
  - Trigger payloads include deterministic correlation metadata (`jobId`, `executionId`, `triggeredAt`).
  - `workflow_start` and `command_invoke` scheduled actions execute through existing routing/validation pathways.
  - Queue lifecycle and failure-requeue semantics are preserved for scheduled executions.
- Automated Test Requirements:
  - Integration tests verifying queue artifacts for scheduled triggers and successful downstream routing.
  - Integration tests validating scheduled `workflow_start` and `command_invoke` behavior.
  - Regression test proving scheduler never bypasses queue/orchestrator path even on retries/failures.
