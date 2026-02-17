# Phase 18 Review: Hardening, Compatibility, and Release Gates

## Findings

### 1. Docs path assertion has a logic hole and does not enforce `config.yaml` references for most targeted spec docs
- Severity: Medium
- File: `tests/docs_compat_and_command_surface.rs:59`
- What is wrong:
  - The assertion currently reads:
    - `text.contains("~/.direclaw/config.yaml") || !spec.ends_with("01-runtime-filesystem.md")`
  - Because three of the four checked specs do **not** end with `01-runtime-filesystem.md`, this condition is automatically true for those files even when `~/.direclaw/config.yaml` is absent.
  - Result: the test can pass while violating P18-T01 acceptance criteria around doc/config-surface alignment.
- Why it matters against the plan/spec:
  - Phase 18 requires docs references to match the implemented config surface and adds documentation integrity checks.
  - This test currently does not actually enforce the intended config-path requirement across the full checked set.
- Action required:
  - Replace the assertion with an unconditional requirement for each listed spec file, or make expectations explicit per file via a table/map of required substrings.
  - Suggested shape:
    - For each spec in the list, assert `contains("~/.direclaw/config.yaml")`.
    - Keep the separate negative assertion banning legacy `~/.direclaw.yaml`.

## Verification Notes
- Reviewed uncommitted changes against `docs/build/config-typing-and-type-driven-setup-tui-plan.md` with Phase 18 focus.
- Executed release gates from `nix-shell`:
  - `cargo fmt --all --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all`
- All commands passed locally.
