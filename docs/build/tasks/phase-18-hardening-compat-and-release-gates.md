# Phase 18: Hardening, Compatibility, and Release Gates

## Goal

Harden the typed config + typed setup architecture, remove transitional debt, and pass full quality gates.

## Tasks

### P18-T01 Complete compatibility audit and docs updates

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Spec example configs and user guide references match implemented config surface.
  - Any retained compatibility shims are documented with removal criteria.
  - Migration guidance is documented for any behavior that changed.
- Automated Test Requirements:
  - Integration tests for all maintained config examples.
  - Documentation checks for referenced file paths/command surfaces.

### P18-T02 Remove dead code and consolidate typed helpers

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Legacy parsing/mutation helpers superseded by typed model are removed.
  - Setup/config modules avoid duplicate validation and conversion logic.
  - Lint-clean codebase with no warnings.
- Automated Test Requirements:
  - Unit tests updated to use final typed APIs.
  - Full clippy run with `-D warnings` passes.

### P18-T03 Execute full release-quality validation from nix shell

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `cargo fmt --all` passes.
  - `cargo clippy --all-targets --all-features -- -D warnings` passes.
  - `cargo test --all` passes.
  - Typed config/setup changes do not regress runtime queue/orchestrator/workflow behavior.
- Automated Test Requirements:
  - CI-equivalent local run of format/lint/test commands from `nix-shell`.
  - Targeted integration run for setup + workflow execution paths.

## Scope Notes
- 2026-02-14: Removed legacy typed-config compatibility shims (`workflow.inputs` mapping object parsing and run metadata `run.json` mirror/fallback), aligned spec/docs references to `~/.direclaw/config.yaml` surfaces, and added docs command/path integrity tests.
