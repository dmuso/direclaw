# Phase 08: Migration, Hardening, and Release Readiness

## Goal

Deliver migration tooling, reliability guarantees, and complete automated coverage before first stable release.

## Tasks

### P08-T01 Implement legacy migration command and fixtures

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `direclaw migrate` upgrades legacy queue/config/path layouts to current schema.
  - Existing queue payload formats and file-tag conventions remain valid after migration.
  - Legacy isolated-workspace configs migrate with zero shared grants by default.
- Automated Test Requirements:
  - Unit tests for migration transforms.
  - Integration tests using representative legacy fixture sets.

### P08-T02 Implement update/rollback safety and operational resilience checks

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `update` supports release checks and rollback on failed transitions.
  - Worker restarts safely handle partially processed queue files.
  - Failure paths are logged with actionable diagnostics and no silent drops.
- Automated Test Requirements:
  - Unit tests for update state transitions and rollback triggers.
  - Integration tests for crash/restart recovery and queue durability guarantees.

### P08-T03 Build required automated test suite matrix

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Unit, integration, adapter, and E2E smoke suites cover requirements in `docs/build/spec/12-reliability-compat-testing.md`.
  - Reliability requirements are mapped to explicit test cases.
  - CI gates releases on passing suites and migration verification.
- Automated Test Requirements:
  - Meta-test/checklist test ensures every reliability requirement has at least one automated test mapping.
  - End-to-end smoke test validates daemon start, workflow execution, and diagnostics response path.
