# Phase 09 Review: Workflow Engine Skeleton and Run Model

## Findings

1. **High** - Workflow execution currently advances only one step per `start`, leaving multi-step runs in `running` until another explicit call to engine APIs.
   - Why this needs actioning:
     - `P09-T03` expects `workflow_start`/`workflow run` to produce runs that can execute steps without manual intervention.
     - Current behavior executes exactly one step attempt and checkpoints the next step, but does not continue automatically.
   - Evidence:
     - `WorkflowEngine::start` calls `execute_or_fail` once: `src/orchestrator.rs:970`
     - `execute_or_fail` calls `execute_next` once and returns: `src/orchestrator.rs:1109`
     - `execute_next` performs one attempt then exits after setting `current_step_id` for next step: `src/orchestrator.rs:1007`
     - Call sites invoke only `start` (no follow-up loop/resume scheduling): `src/cli.rs:1691`, `src/orchestrator.rs:3016`
   - Recommended fix:
     - Add an engine loop entrypoint (or loop in `start`/`resume`) that repeatedly executes steps until terminal state or `waiting`.
     - Keep per-attempt checkpoints/logging as-is between iterations.
     - Add an integration test asserting a multi-step workflow reaches terminal (or waiting) state from a single `workflow run`/`workflow_start` trigger.

## Verification Notes

- Reviewed uncommitted changes in:
  - `src/cli.rs`
  - `src/orchestrator.rs`
  - `tests/cli_command_surface.rs`
  - `tests/orchestrator_workflow_engine.rs`
- Validation run:
  - `nix-shell --run "cargo test --all"` (pass)
