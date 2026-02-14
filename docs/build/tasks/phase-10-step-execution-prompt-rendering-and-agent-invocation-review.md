# Phase 10 Review: Step Execution, Prompt Rendering, and Agent Invocation

## Findings (Action Required)

1. **High: Workspace deny checks happen after filesystem side effects in step execution path**
- `execute_step_attempt` creates `run_workspace` and `agent_workspace` directories before `enforce_workspace_access` runs.
- References: `src/orchestrator.rs:1315`, `src/orchestrator.rs:1331`, `src/orchestrator.rs:1333`.
- Impact: A denied/unauthorized workspace path can still be created on disk before the run is failed. This violates the Phase-10 expectation that denied workspace access fails safely before execution side effects.
- Required action:
  - Move workspace-access enforcement before any `create_dir_all` calls for run/agent workspaces.
  - Add a regression test that asserts denied paths are **not** created when access is rejected.

2. **High: `workflow run` CLI path bypasses workspace-access enforcement context**
- CLI `workflow run` constructs `WorkflowEngine` without `with_workspace_access_context(...)`.
- Reference: `src/cli.rs:1689`.
- Impact: Direct CLI-triggered runs do not apply the same workspace isolation/grant enforcement used by the queue/orchestrator runtime path, conflicting with the “every step run” enforcement requirement in Phase 10 and project constraints.
- Required action:
  - Resolve workspace context via settings in the CLI path and pass it into the engine.
  - Reuse existing enforcement helpers so CLI and queue paths enforce identical workspace policy.
  - Add a CLI/integration test for a denied agent workspace configuration on `workflow run`.

## Validation Notes
- Reviewed uncommitted diffs in:
  - `src/orchestrator.rs`
  - `src/runtime.rs`
  - `tests/orchestrator_workflow_engine.rs`
  - `tests/message_flow_queue_orchestrator_provider_e2e.rs`
  - `tests/cli_command_surface.rs`
- Executed targeted suites in `nix-shell`:
  - `cargo test --test orchestrator_workflow_engine --test message_flow_queue_orchestrator_provider_e2e --test cli_command_surface`
  - Current tests pass, but they do not currently guard the two issues above.
