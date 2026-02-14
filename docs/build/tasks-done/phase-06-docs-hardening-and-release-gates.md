# Phase 06: Documentation, Hardening, and Release Gates

## Goal

Finalize end-user documentation, operational hardening, and enforce release blockers so v1 ships with confidence.

## Plan Context

Primary reference:
- `docs/build/release-readiness-plan.md`

## Tasks

### P06-T01 Deliver complete user guide for first-time operators

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Expand `docs/user-guide/README.md` into a full navigation entrypoint.
  - Add step-by-step install and first-run walkthrough for release binaries.
  - Include Slack setup and auth sync integration points.
  - Add troubleshooting matrix for common startup/runtime failures.
- Acceptance Criteria:
  - New user can install binary and complete first message flow using only docs.
- Automated Test Requirements:
  - Manual-doc-backed acceptance test script executed in CI-like clean environment.

### P06-T02 Add operator runbook for production usage

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Document service management patterns (systemd/launchd examples).
  - Document log locations, backup strategy, and incident procedures.
  - Add upgrade and rollback guidance for post-v1 releases.
- Acceptance Criteria:
  - Operator can run and maintain service without codebase knowledge.
- Automated Test Requirements:
  - Docs link/command snippet validation for runbook commands and paths.

### P06-T03 Add project hygiene and release governance files

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Add:
    - `LICENSE`
    - `CHANGELOG.md`
    - `CONTRIBUTING.md`
    - `SECURITY.md`
  - Ensure README links to these files.
- Acceptance Criteria:
  - Repository has standard release and contribution governance artifacts.
- Automated Test Requirements:
  - CI check verifies presence and non-empty content for required governance files.

### P06-T04 Implement release blocker checklist enforcement

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Add a release checklist document aligned to plan go/no-go criteria.
  - Add CI gate script to verify:
    - tests pass
    - release artifacts present for matrix
    - docs checks pass
    - no placeholder operational responses remain
- Acceptance Criteria:
  - `v1.0.0` cannot be cut unless all release blockers pass.
- Automated Test Requirements:
  - Pipeline test proving gate fails when any blocker condition is intentionally violated.

### P06-T05 Final release candidate E2E validation

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Run clean-environment scenario:
    - install binary
    - `setup`
    - configure Slack profile
    - start runtime
    - process real inbound test message to outbound response
  - Capture artifacts as release evidence under `docs/build/review/actioned`.
- Acceptance Criteria:
  - Final release candidate demonstrates full end-to-end user workflow.
- Automated Test Requirements:
  - Automated smoke suite and evidence artifact generation in CI.
