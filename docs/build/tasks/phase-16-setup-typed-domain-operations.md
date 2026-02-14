# Phase 16: Setup Typed Domain Operations

## Goal

Refactor setup mutation logic to typed domain operations so config edits are consistent, composable, and testable.

## Tasks

### P16-T01 Introduce `SetupState` domain aggregate and typed mutation API

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Setup bootstrap state is represented by a dedicated typed aggregate.
  - Typed mutation functions exist for orchestrator/workflow/step/agent edits.
  - Mutations enforce invariants at mutation boundaries (not only at final save).
- Automated Test Requirements:
  - Unit tests for mutation API success/failure paths.
  - Unit tests for invariants (default workflow, non-empty workflow list, id uniqueness).

### P16-T02 Migrate orchestrator/workflow/step/agent setup flows to typed operations

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Setup key handlers call typed domain operations instead of deep direct mutation.
  - Duplicate update logic across setup menus is removed.
  - Existing setup UX behavior is preserved unless explicitly documented.
- Automated Test Requirements:
  - Integration tests for add/edit/delete flows across orchestrators, workflows, steps, and agents.
  - Regression tests for setup save and reload parity.

### P16-T03 Ensure persistence boundary is typed and validated

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Save path serializes typed settings/orchestrator config only after full validation pass.
  - Canonical orchestrator config persistence path remains enforced.
  - Setup error messages stay user-oriented and specific.
- Automated Test Requirements:
  - Integration tests for setup save failures with invalid state.
  - Integration tests asserting canonical config file creation/update behavior.
