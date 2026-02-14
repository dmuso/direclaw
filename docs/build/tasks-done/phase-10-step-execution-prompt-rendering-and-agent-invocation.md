# Phase 10: Step Execution, Prompt Rendering, and Agent Invocation

## Goal
Implement provider-backed step execution with prompt/context rendering so workflow steps actually run using configured agents and persisted workflow inputs.

## Dependencies
- `docs/build/spec/03-agent-routing-execution.md`
- `docs/build/spec/05-workflow-orchestration.md`
- `docs/build/spec/06-provider-integration.md`
- `docs/build/workflow-system-implementation-plan.md`

## Tasks

### P10-T01 Implement deterministic step prompt renderer

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Renderer supports workflow/input placeholders used by orchestrator specs.
  - Missing required placeholders fail fast with explicit error.
  - Rendered prompt/context artifacts are persisted per attempt.
- Verification:
  - Add unit tests for placeholder substitution and missing-key failures.
  - Add fixture-driven tests for engineering/product sample prompt patterns.
  - Run `nix-shell --run "cargo test --all"`.

### P10-T02 Execute step attempts through provider runner for step agents

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Engine resolves step agent and invokes configured provider/model in correct workspace.
  - Provider invocation logs are persisted per step attempt.
  - Non-zero/timeout/parse provider failures are propagated into run state transitions.
- Verification:
  - Add integration tests with mocked provider scripts for success/failure/timeout.
  - Add assertions for attempt log files under run step attempt directories.
  - Run `nix-shell --run "cargo test --test provider_runner --test orchestrator_workflow_engine"`.

### P10-T03 Parse and validate `[workflow_result]` envelope in live step path

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Every successful step attempt requires valid workflow envelope JSON object.
  - Envelope parse errors trigger retry/failure rules based on configured limits.
  - Parsed outputs are persisted in attempt result records.
- Verification:
  - Add unit tests for malformed envelope edge cases.
  - Add integration test proving malformed output increments attempt and respects retry limits.
  - Run `nix-shell --run "cargo test --all"`.

### P10-T04 Validate workspace access and execution isolation for every step run

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Step execution enforces orchestrator private workspace and shared grants before provider execution.
  - Denied workspace access is logged to security log and fails run safely.
  - Execution never proceeds with unresolved/invalid workspace roots.
- Verification:
  - Add integration tests covering allow/deny workspace scenarios for step execution path.
  - Confirm security log artifact is emitted on deny.
  - Run `nix-shell --run "cargo test --test workspace_access --test orchestrator_workflow_engine"`.

