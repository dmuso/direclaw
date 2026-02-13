# Phase 03: Orchestrator Routing and Workflow Engine

## Goal

Deliver orchestrator-first routing, selector-driven action resolution, workflow state machine, and deterministic step execution.

## Tasks

### P03-T01 Implement channel-profile to orchestrator routing and selector I/O persistence

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Channel-originated messages resolve `orchestrator_id` via `channel_profiles`.
  - Selector request/result artifacts persist to required orchestrator message/select paths.
  - Invalid selector output triggers retry and falls back to `default_workflow` after retry limit.
- Automated Test Requirements:
  - Unit tests for orchestrator/profile resolution and selector schema validation.
  - Integration test for selector success, retry, and default-workflow fallback behavior.

### P03-T02 Implement selector action routing (`workflow_start|workflow_status|command_invoke`)

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `workflow_start` launches selected valid workflow.
  - `workflow_status` resolves run by precedence and returns progress without advancing steps.
  - `command_invoke` executes only functions exposed in `availableFunctions`; unknown functions are rejected.
- Automated Test Requirements:
  - Unit tests for action validation and function registry lookup.
  - Integration tests for status and command paths proving no step advancement.

### P03-T03 Implement workflow run state model and progress snapshots

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Run and step state persist under `~/.direclaw/workflows/runs/<run_id>`.
  - `progress.json` is updated at required lifecycle points.
  - Valid run states are enforced (`queued|running|waiting|succeeded|failed|canceled`).
- Automated Test Requirements:
  - Unit tests for state transition guards.
  - Integration test of multi-step workflow with persisted state/progress artifacts.

### P03-T04 Implement workflow step execution contract and deterministic routing rules

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Step attempts parse required `[workflow_result]` JSON envelope.
  - `agent_task` and `agent_review` transitions follow deterministic routing rules.
  - Retry limits, run timeout, step timeout, and max-iteration safety controls are enforced.
- Automated Test Requirements:
  - Unit tests for envelope parsing and review decision parsing.
  - Integration test covering approve/reject loops, retry exhaustion, and timeout termination.
