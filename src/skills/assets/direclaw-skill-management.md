---
name: direclaw-skill-management
description: "Use when creating, updating, or reviewing workspace skill files used by Codex or Claude."
---

# DireClaw Skill Management

## Purpose
Manage skill files only.

## DireClaw Context
- Skills are Markdown files mounted into tool environments.
- Canonical location is `skills/<skill-name>/SKILL.md` inside the orchestrator private workspace.
- A skill should be focused on one job with a clear trigger description.
- Reliable execution improves when skills include concrete steps, boundaries, and deterministic deliverables.

## What To Do
1. Create or edit only the target skill file(s).
2. Keep frontmatter `name` and `description` precise and unambiguous.
3. Keep instructions concrete, deterministic, and scoped to a single responsibility.
4. Keep DireClaw-specific context in the skill when domain behavior is non-obvious.
5. Include explicit sections that reduce ambiguity:
   - Purpose
   - DireClaw Context
   - What To Do
   - Where To Change/Check
   - How To Verify
   - What Not To Do
   - Deliverable

## Where To Change
- Target skill files under:
  - `<current_working_directory>/skills/<skill-name>/SKILL.md`

## How To Verify
1. Confirm frontmatter fields are present and correct.
2. Confirm instructions are deterministic and free of contradictory guidance.
3. Confirm skill scope is single-purpose and does not overlap unrelated config edits.
4. Confirm output expectations are explicit and file/path-based when relevant.

## What Not To Do
- Do not edit agent definitions in this skill.
- Do not conflate skill content changes with orchestrator config changes.
- Do not add ambiguous "required inputs" language as if runtime-enforced.
- Do not leave deliverables implicit.

## Deliverable
Return:
- Skill files changed.
- Purpose/trigger clarity improvements made.
- Any follow-up needed for related config or workflow references.
