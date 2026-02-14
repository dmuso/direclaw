# Phase 00: Spec Closure

## Goal

Resolve open spec decisions and lock implementation defaults before coding runtime behavior.

## Tasks

### P00-T01 Decide unresolved options in decision workbook

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Every decision section in `docs/build/spec/13-decision-workbook.md` has `Status: decided`.
  - Each decision includes chosen option and rationale.
  - Way-forward fields include concrete code and test follow-ups.
- Automated Test Requirements:
  - Doc lint/check test verifies no section remains with `Status: open`.

### P00-T02 Patch normative specs with decided behavior

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Resolved behavior is reflected in `docs/build/spec/01-runtime-filesystem.md`, `docs/build/spec/05-workflow-orchestration.md`, `docs/build/spec/06-provider-integration.md`, `docs/build/spec/07-channel-adapters.md`, and `docs/build/spec/10-daemon-operations.md`.
  - Removed ambiguities are replaced with deterministic rules.
- Automated Test Requirements:
  - Spec consistency test checks that decided options and normative specs do not conflict.

### P00-T03 Freeze examples to match final specs

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Example settings/orchestrator/prompt files under `docs/build/spec/examples` are updated for decision-aligned fields and defaults.
  - Example configs validate against current schema checks.
- Automated Test Requirements:
  - Example validation test parses all example YAML/prompt templates and asserts schema compliance.

DONE.
