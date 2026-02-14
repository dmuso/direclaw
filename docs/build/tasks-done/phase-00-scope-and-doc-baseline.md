# Phase 00: Scope and Documentation Baseline

## Goal

Lock v1 scope, remove spec/documentation ambiguity, and establish traceability before implementation-heavy phases.

## Plan Context

Primary reference:
- `docs/build/release-readiness-plan.md`

## Tasks

### P00-T01 Lock v1 product scope and deferred items

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Update README and user-facing docs to explicitly state v1 channel support (Slack-only).
  - Add a short "Deferred After v1" section listing Discord/Telegram/WhatsApp as roadmap items.
  - Ensure no doc claims unsupported v1 capabilities as currently available.
- Acceptance Criteria:
  - README and docs consistently describe v1 in-scope and out-of-scope features.
  - No contradictory statements across `README.md`, `docs/user-guide/*`, and build-spec references.
- Automated Test Requirements:
  - Add a docs consistency check (script/test) that scans for forbidden unsupported-v1 claims.

### P00-T02 Resolve canonical spec source-of-truth mismatch

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Choose one canonical spec path for this repository.
  - Update references in README, task files, and internal docs to use only the canonical path.
  - If compatibility links are needed, add redirect notes or mirrored index stubs.
- Acceptance Criteria:
  - All active references resolve to existing files.
  - `AGENTS.md`, README, and task docs use canonical `docs/build/spec/*` paths.
- Automated Test Requirements:
  - Add link-check test for markdown docs used by contributors/operators.

### P00-T03 Create requirement traceability index

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Add a traceability table mapping:
    - Plan requirement -> phase task -> tests (file names/test IDs).
  - Keep the table in `docs/build/review` or `docs/build/tasks` as a machine-readable markdown artifact.
- Acceptance Criteria:
  - Every release-blocking requirement in the plan has at least one owning task and test reference.
- Automated Test Requirements:
  - Add a validation script/test that fails if any release-blocking requirement has no mapped task/test.

### P00-T04 Establish task execution workflow for contributors

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Update `docs/build/tasks/README.tasks.md` with explicit "how to execute and update statuses" guidance.
  - Add a short "definition of complete" checklist for each task.
- Acceptance Criteria:
  - A junior engineer can follow the workflow and update statuses without additional guidance.
- Automated Test Requirements:
  - Add doc lint rule verifying each phase file contains status tokens, acceptance criteria, and automated test sections.
