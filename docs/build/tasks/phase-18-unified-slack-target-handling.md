# Phase 18: Unified Slack Target Handling

## Goal
Implement one unified Slack target resolution and delivery path for all request contexts (ad-hoc chat requests, workflow actions, command invocations, and scheduled jobs) while keeping scheduler core channel-agnostic and adapter-owned for transport details. See `docs/build/spec/07-channel-adapters.md` for overall channel adapter related specs.

## Tasks

### P18-T01 Define and validate Slack target reference contract

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - A typed Slack `targetRef` contract is defined for all Slack-bound actions (channel id, optional thread id, posting mode).
  - Validation enforces required Slack target fields regardless of entrypoint (`workflow_start`, `command_invoke`, ad-hoc orchestrator action, or scheduled trigger).
  - Invalid or incomplete Slack target references fail fast with actionable validation errors.
  - Contract documentation is aligned across scheduler, orchestration/routing, and channel adapter specs.
- Automated Test Requirements:
  - Unit tests for Slack `targetRef` schema validation and normalization.
  - Unit tests for error messaging on missing/invalid target fields.
  - Regression tests ensuring non-Slack actions are not constrained by Slack-only target rules.

### P18-T02 Add orchestrator-safe mapping from all request contexts to Slack delivery context

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Slack-bound actions resolve required `channelProfileId` and orchestrator scope deterministically for every entrypoint.
  - Mismatched profile/orchestrator mappings are rejected before enqueue.
  - Payloads carry Slack target metadata without bypassing orchestrator routing.
  - Audit logs include resolved profile id and target channel/thread metadata for each Slack-bound dispatch attempt.
- Automated Test Requirements:
  - Integration tests for valid and invalid profile-orchestrator mapping across ad-hoc, workflow, command, and scheduled paths.
  - Integration tests for payload propagation of Slack target metadata through queue entries for each entrypoint.
  - Regression test verifying cross-orchestrator target leakage is prevented.

### P18-T03 Implement one canonical Slack adapter delivery path for targeted posts

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Slack adapter exposes a single internal delivery path for targeted channel/thread posts used by all upstream contexts.
  - Scheduler, workflow, and ad-hoc command/chat paths reuse the same Slack delivery function and payload contract.
  - Delivery behavior respects Slack allowlist/mention and channel policy constraints where applicable.
  - Adapter reports delivery success/failure with retry-safe error handling and log visibility.
  - Unified targeted posting does not interfere with normal conversation-thread reply behavior.
- Automated Test Requirements:
  - Adapter integration tests for channel post and thread post delivery through the shared path.
  - Integration tests proving each upstream context reaches the same adapter code path.
  - Integration tests for policy enforcement failures (unauthorized channel/invalid target) with deterministic errors.
  - Regression tests proving normal inbound/outbound Slack workflow messaging remains unchanged.

### P18-T04 End-to-end validation across all Slack-target entrypoints

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Natural-language and command-driven ad-hoc targeting can post to allowed Slack channels/threads via orchestrator routing.
  - Scheduled job lifecycle operations (`pause`, `resume`, `delete`, `run_now`) affect Slack-targeted execution as expected.
  - Runtime status/log surfaces expose unified Slack target delivery outcomes across all entrypoints.
  - Reliability baseline includes both scheduled and non-scheduled Slack target scenarios in integration coverage.
- Automated Test Requirements:
  - End-to-end test: NL ad-hoc intent -> selector action/command -> queue -> orchestrator -> Slack targeted outbound success.
  - End-to-end test: NL scheduled intent -> `schedule.create` -> scheduler trigger -> queue -> orchestrator -> Slack targeted outbound success.
  - End-to-end tests for pause/resume/delete/run_now behavioral correctness on Slack-targeted jobs.
  - Regression test for restart + misfire recovery with Slack-targeted scheduled jobs.
