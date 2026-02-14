# Phase 04 Command/Selector Parity and UX Review

## Scope Reviewed
- Uncommitted changes in:
  - `docs/build/tasks/phase-04-command-selector-parity-and-ux.md`
  - `src/cli.rs`
  - `src/orchestrator.rs`
  - `src/runtime.rs`
  - `tests/cli_command_surface.rs`
  - `tests/orchestrator_workflow_engine.rs`
- Baseline requirements:
  - `docs/build/release-readiness-plan.md` (Phase 04 + command parity/update contract)
  - Supporting command-parity contract in `docs/build/spec/09-configuration-cli.md` and `docs/build/spec/10-daemon-operations.md`

## Validation Run
- `nix-shell --run "cargo test --test cli_command_surface --test orchestrator_workflow_engine"` (pass)

## Findings (Needs Action)

1. High: selector function parity is still partial versus the v1 command surface.
- Requirement reference:
  - `docs/build/release-readiness-plan.md:49` requires full command surface stability for v1.
  - `docs/build/release-readiness-plan.md:113` requires completed selector function registry parity for supported v1 commands.
  - `docs/build/spec/10-daemon-operations.md:26` requires every required command to have selector-callable form.
- Current behavior:
  - `FunctionRegistry::v1_catalog()` exposes only 9 function ids (`workflow.list/show/status/progress/cancel`, `orchestrator.list/show`, `channel_profile.list/show`) in `src/orchestrator.rs:1299`.
  - Runtime uses this as the default selector function surface (`src/runtime.rs:1133`).
- Impact:
  - Natural-language `command_invoke` routing cannot reach large parts of the required command surface.
- Action:
  - Expand catalog + invocation handlers to cover the supported v1 command set (including missing channel-profile parity functions and required daemon/config command forms), then add parity tests that assert registry completeness against the spec command list.

2. Medium: `doctor` binary checks can report false positives for non-executable files.
- Requirement reference:
  - `docs/build/release-readiness-plan.md:115` requires reliable install/runtime diagnostics.
- Current behavior:
  - `is_binary_available` only checks `is_file()` on PATH candidates (`src/cli.rs:761`) and does not verify executability.
- Impact:
  - `summary=healthy` may be reported even when binaries exist but cannot execute.
- Action:
  - Validate executability (or execute a lightweight `--version` probe) before marking binary checks as `ok`.
  - Add a test case where `claude`/`codex` files exist without execute permission and must fail diagnostics.

3. Medium: CLI output consistency acceptance is not fully evidenced by tests.
- Requirement reference:
  - `docs/build/release-readiness-plan.md:114` calls for strengthened output consistency and operational error quality.
  - Task acceptance requires CLI output snapshots for critical commands (`docs/build/tasks/phase-04-command-selector-parity-and-ux.md`).
- Current behavior:
  - Added tests assert selected substrings for `update` and `doctor`, but there are no snapshot/contract assertions for a standardized output shape across critical commands (`tests/cli_command_surface.rs:100`, `tests/cli_command_surface.rs:114`).
- Impact:
  - Regressions in operator-facing output format and remediation quality can slip through without detection.
- Action:
  - Add snapshot/contract tests for key operational commands (`status`, `update`, `doctor`, and representative failure paths) that assert both structure and remediation hints.
