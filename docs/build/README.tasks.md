# Release Readiness Task Phases

Plan context:
- `docs/build/release-readiness-plan.md`

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
