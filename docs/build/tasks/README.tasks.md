# Implementation Phases

Use one phase file at a time and move each task `Status` through:

- `todo`
- `in_progress`
- `complete`

Rules:
- Do not mark a task `complete` until all acceptance criteria and automated test requirements are satisfied.
- Keep task status current in the phase file as work progresses.
- If scope changes, append a dated "Scope Notes" section to the affected phase file.

Phase files:

1. `docs/build/tasks/phase-14-config-typing-foundations.md`
2. `docs/build/tasks/phase-15-typed-workflow-inputs-and-output-contracts.md`
3. `docs/build/tasks/phase-16-setup-typed-domain-operations.md`
4. `docs/build/tasks/phase-17-type-driven-setup-tui-navigation.md`
5. `docs/build/tasks/phase-18-hardening-compat-and-release-gates.md`
