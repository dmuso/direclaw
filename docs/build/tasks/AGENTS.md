# AGENTS: Task Doc Standards (`docs/build/tasks`)

This file defines the required structure and conventions for task documents in this folder.

## Purpose
Use these task docs to track executable implementation work with clear acceptance gates and status progression.

## Required Status Lifecycle
Every task item must include:

- `Status: todo` (`todo|in_progress|complete`) at creation time

Allowed values:
- `todo`
- `in_progress`
- `complete`

Rules:
- Move `todo -> in_progress -> complete`.
- Do not mark `complete` unless all acceptance criteria and automated test requirements are satisfied.
- Keep status current in the same file where the task is defined.

## Required File Structure
Each phase file must follow this shape:

1. `# Phase NN: <Title>`
2. `## Goal`
3. `## Tasks`
4. One or more task entries using this template:

```md
### PNN-TMM <Task title>

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - <criterion>
  - <criterion>
- Automated Test Requirements:
  - <test requirement>
  - <test requirement>
```

## Naming Conventions
- Phase files: `phase-<number>-<kebab-title>.md`
- Task IDs: `P<phase>-T<task>` (example: `P14-T02`)
- Keep task IDs stable after creation.

## Writing Standards
- Tasks must be executable and scoped to an implementable slice.
- Acceptance criteria must describe observable outcomes, not implementation intent only.
- Automated test requirements must be explicit (unit/integration/regression) and relevant to the task.
- Use concise, deterministic language.

## Scope Change Handling
If phase scope changes, append a section at the end of the phase file:

```md
## Scope Notes
- YYYY-MM-DD: <what changed and why>
```

Do not rewrite historical task intent; append notes instead.

## Completion Gate
Before a phase is considered done:
- All tasks in that phase are `complete`.
- Required tests listed in tasks are implemented and passing.
- Repo quality gates pass per project standards (fmt, clippy, test) in the required environment.
