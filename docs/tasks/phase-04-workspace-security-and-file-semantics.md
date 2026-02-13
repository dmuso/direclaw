# Phase 04: Workspace Security and File Semantics

## Goal

Enforce workspace isolation and path safety, and implement deterministic inbound/outbound file tag behavior.

## Tasks

### P04-T01 Implement pre-execution workspace access enforcement

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Runtime evaluates private + allowed shared workspace context before provider execution.
  - Ungranted shared path access attempts are rejected and logged.
  - Orchestrators with no grants can only access private workspace.
- Automated Test Requirements:
  - Unit tests for shared access subset/allowlist checks.
  - Integration tests validating allow/deny behavior across orchestrators.

### P04-T02 Implement output path safety for workflow output files

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Output paths are precomputed from `output_files` templates per attempt.
  - All resolved output paths stay under step output root.
  - Traversal or non-canonical output path attempts fail step validation.
- Automated Test Requirements:
  - Unit tests for output template interpolation and canonical-path checks.
  - Integration test with malicious path input asserting block-and-log behavior.

### P04-T03 Implement file tag parsing, send-file stripping, and truncation contract

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Inbound file tags use absolute paths and are preserved in queue payload semantics.
  - Outbound `[send_file: ...]` tags are extracted to `files[]` and removed from user-visible text.
  - Post-strip truncation enforces 4000 max chars with exact truncation suffix contract.
- Automated Test Requirements:
  - Unit tests for deterministic file tag extraction/stripping.
  - Integration test for file round-trip from inbound adapter event to outbound delivery payload.
