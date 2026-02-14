# Release Readiness Task Phases

Plan context:
- `docs/build/release-readiness-plan.md`

## Execution Workflow

1. Pick exactly one phase file and set one task status to `in_progress`.
2. Read the plan plus relevant spec docs under `docs/build/spec/`.
3. Implement the smallest coherent slice that satisfies the task acceptance criteria.
4. Add or update automated tests required by the task.
5. Run validation in `nix-shell`:
   - `cargo fmt --all`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --all`
6. Update task status to `complete` only after all checks pass.
7. Commit with a message that includes the phase task id (for example `P01-T02`).

Use one phase file at a time and move each task `Status` through:

- `todo`
- `in_progress`
- `complete`

Phase files:

1. `docs/build/tasks/phase-00-scope-and-doc-baseline.md`
2. `docs/build/tasks/phase-01-runtime-supervisor-and-workers.md`
3. `docs/build/tasks/phase-02-queue-orchestrator-provider-e2e.md`
4. `docs/build/tasks/phase-03-slack-runtime-worker.md`
5. `docs/build/tasks/phase-04-command-selector-parity-and-ux.md`
6. `docs/build/tasks/phase-05-github-release-automation.md`
7. `docs/build/tasks/phase-06-docs-hardening-and-release-gates.md`

Implementation expectations:

- Read plan + relevant spec sections before coding.
- Keep changes small and test-backed.
- For each task:
  - satisfy acceptance criteria,
  - satisfy automated test requirements,
  - update task status.

## Definition Of Complete Checklist

Apply this checklist to every task before marking it `complete`:

- Acceptance criteria are met in behavior and docs.
- Automated test requirements are implemented and passing.
- Related docs are updated for any command/config/runtime changes.
- No placeholder, misleading, or contradictory output remains.
- Status line in the phase file is updated to `complete`.
