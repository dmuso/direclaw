# Phase 05: GitHub Release Automation

## Goal

Implement CI and release automation to produce reliable, distributable binaries for end users.

## Plan Context

Primary reference:
- `docs/build/release-readiness-plan.md`

## Tasks

### P05-T01 Implement required CI workflows

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Add PR CI workflow in `.github/workflows`.
  - Run required checks:
    - `cargo fmt --all -- --check`
    - `cargo clippy --all-targets --all-features -- -D warnings`
    - `cargo test --all`
  - Add docs link/check step.
- Acceptance Criteria:
  - CI fails on formatting, lint, test, or docs-link regressions.
- Automated Test Requirements:
  - Workflow-level validation by running on sample PR branch.

### P05-T02 Implement release build matrix and artifact packaging

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Build binaries for:
    - macOS arm64
    - Linux x86_64
    - Linux arm64
  - Package each artifact in versioned tarballs.
  - Produce `checksums.txt`.
- Acceptance Criteria:
  - Tagging release branch creates full artifact set with deterministic filenames.
- Automated Test Requirements:
  - Release workflow test run validating artifact names and checksum generation.

### P05-T03 Publish GitHub Releases with validated notes

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - Create release notes template including install steps and known limits.
  - Attach all artifacts and checksums to GitHub Release.
  - Include SHA256 verification instructions.
- Acceptance Criteria:
  - End users can install from release page without source build.
- Automated Test Requirements:
  - Smoke test installs from generated artifacts and runs core commands (`setup`, `status`, `--help`).

### P05-T04 Define update command behavior aligned with release system

- Status: `complete` (`todo|in_progress|complete`)
- Implementation Notes:
  - `update check` must query real release metadata.
  - `update apply` must be either implemented safely or fail explicitly as unsupported.
  - Remove static hardcoded `latest_version` responses.
- Acceptance Criteria:
  - Update outputs reflect real release state and never mislead users.
- Automated Test Requirements:
  - Unit/integration tests for update metadata parsing and unsupported/apply behavior.
