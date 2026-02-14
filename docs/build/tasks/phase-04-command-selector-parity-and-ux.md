# Phase 04: Command/Selector Parity and UX Quality

## Goal

Complete selector command-invoke coverage for supported v1 operations and improve operational CLI experience.

## Plan Context

Primary reference:
- `docs/build/release-readiness-plan.md`

## Tasks

### P04-T01 Expand function registry for command parity (v1 scope)

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Define function IDs for supported v1 commands (workflow, orchestrator, channel-profile, status/progress actions).
  - Keep naming stable and command-aligned.
  - Include argument schemas and short descriptions for selector disambiguation.
- Acceptance Criteria:
  - Selector receives complete machine-readable function metadata for supported v1 commands.
  - Unknown functions are rejected deterministically.
- Automated Test Requirements:
  - Unit tests for function schema generation and registry completeness.
  - Integration tests for `command_invoke` allowlist enforcement.

### P04-T02 Harden command invocation validation and safety

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Validate required args, type constraints, and reject unknown arg keys.
  - Ensure read-only commands (status/progress) never mutate state.
  - Return structured, user-safe errors for invalid invocations.
- Acceptance Criteria:
  - Command invocations are deterministic and safe for chat-originated usage.
- Automated Test Requirements:
  - Contract tests for required/missing/invalid args and read-only guarantees.

### P04-T03 Improve CLI operational output quality

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Standardize success/error output shape across commands.
  - Ensure high-signal error messages include concrete remediation hints.
  - Remove placeholder or misleading outputs (especially update-related commands).
- Acceptance Criteria:
  - Operators can act on CLI errors without reading source code.
  - No command reports fake success for unsupported behavior.
- Automated Test Requirements:
  - CLI integration tests with output snapshots for critical commands.

### P04-T04 Implement runtime readiness diagnostics command

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Add `direclaw doctor` to validate environment prerequisites:
    - config paths
    - required binaries
    - required env vars
    - workspace permissions
  - Output both summary and detailed findings.
- Acceptance Criteria:
  - Fresh installs can run one command to identify setup blockers.
- Automated Test Requirements:
  - Integration tests for healthy and unhealthy environment permutations.

