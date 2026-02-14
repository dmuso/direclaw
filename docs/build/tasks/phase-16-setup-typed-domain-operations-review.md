# Phase 16 Review: Setup Typed Domain Operations

## Findings (Action Required)

1. High: Mutation-boundary invariants are still deferred to save-time for several typed operations.
`SetupState` accepts mutations that can produce orchestrator configs that fail `OrchestratorConfig::validate(...)`, which conflicts with the phase goal to enforce invariants at mutation boundaries.
- `set_step_agent(...)` does not verify that the target agent exists (`src/cli/setup_tui.rs:645`).
- `set_step_outputs(...)` and `set_step_output_files(...)` allow empty contracts even though config validation requires non-empty `outputs` and `output_files` (`src/cli/setup_tui.rs:742`, `src/cli/setup_tui.rs:756`, `src/config.rs:957`, `src/config.rs:963`).
- `toggle_agent_orchestration_capability(...)` allows turning off orchestration for the selector agent, but selector capability is required by validation (`src/cli/setup_tui.rs:910`, `src/config.rs:895`).
Action:
- Enforce these checks directly in the typed mutation methods (or call a stronger invariant validator after each relevant mutation).
- Update UI status paths so invalid edits are rejected immediately instead of failing only on save.

2. Medium: Remove operations can return success when the target entity does not exist.
These methods currently perform a no-op and still return `Ok(())` if the specified id is absent, which can mislead the setup UI and tests.
- `remove_orchestrator(...)` (`src/cli/setup_tui.rs:172`)
- `remove_workflow(...)` (`src/cli/setup_tui.rs:409`)
- `remove_step(...)` (`src/cli/setup_tui.rs:576`)
- `remove_agent(...)` (`src/cli/setup_tui.rs:817`)
Action:
- Explicitly check existence first and return a clear user-facing error when the id is missing.

3. Medium: Phase-16 integration test coverage is incomplete relative to the task requirements.
The new integration suite covers canonical orchestrator path persistence and save-boundary failure (`tests/setup_typed_domain_operations.rs:15`, `tests/setup_typed_domain_operations.rs:49`), but does not cover add/edit/delete flows across orchestrators, workflows, steps, and agents as required by P16-T02.
Action:
- Add integration tests that exercise typed setup operations for add/edit/delete across all four entity levels, plus save+reload parity for those edits.
