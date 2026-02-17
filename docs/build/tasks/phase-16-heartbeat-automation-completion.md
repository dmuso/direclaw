# Phase 16: Heartbeat Automation Completion

## Goal
Implement the remaining heartbeat automation behavior so the heartbeat worker performs per-agent heartbeat enqueueing, response observation, and logging per spec. See `docs/build/spec/11-heartbeat-service.md` for spec.

## Tasks

### P16-T01 Implement per-agent heartbeat enqueue pipeline

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Heartbeat worker tick resolves all configured orchestrators and their configured agents.
  - For each agent, worker loads `<agent_dir>/heartbeat.md` when present and uses a deterministic fallback prompt when missing.
  - Worker enqueues one heartbeat message per agent into the resolved orchestrator queue with deterministic correlation metadata.
  - A missing `heartbeat.md` file never aborts the full tick cycle.
- Automated Test Requirements:
  - Unit test coverage for prompt resolution (`heartbeat.md` present and missing).
  - Unit test coverage for queue payload construction and deterministic message id/correlation fields.
  - Integration test that one tick produces heartbeat queue entries for all configured agents across multiple orchestrators.

### P16-T02 Implement heartbeat response matching and monitoring logs

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Worker inspects outbound queue for responses matching heartbeat correlation ids.
  - Matching response snippets are logged with orchestrator id, agent id, and heartbeat message id.
  - Missing outbound response is logged as a non-fatal monitoring event.
  - Response matching does not consume or mutate outbound files used by adapter delivery.
- Automated Test Requirements:
  - Unit test coverage for outbound response matcher (match, no match, malformed payload).
  - Integration test validating logs include matched response snippet metadata.
  - Regression test proving outbound files remain available for normal adapter processing.

### P16-T03 Integrate heartbeat automation with runtime reliability and status surfaces

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Heartbeat worker failures are reported through worker health state and runtime logs without crashing supervisor.
  - Worker honors enable/disable semantics from `monitoring.heartbeat_interval`.
  - Runtime status output reflects active heartbeat worker liveness through `last_heartbeat`.
  - Heartbeat automation behavior is documented and linked from spec index/reliability checks where required.
- Automated Test Requirements:
  - Integration test for enabled and disabled heartbeat worker startup behavior.
  - Integration test injecting heartbeat tick failure and asserting non-fatal supervisor continuity.
  - Regression test that `status` includes heartbeat worker state/last heartbeat updates during runtime.
