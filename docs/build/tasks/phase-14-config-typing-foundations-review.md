# Phase 14 Review: Config Typing Foundations

## Findings (Needs Action)

1. **Medium**: Spec-orchestrator compatibility test does not exercise typed validation path.
- Evidence: `tests/config_typing_foundations.rs:84` parses orchestrator examples, but the test never calls `OrchestratorConfig::validate(...)`; it only pattern-matches provider enums and then drops settings at `tests/config_typing_foundations.rs:100` and `tests/config_typing_foundations.rs:105`.
- Why this matters: P14-T03 acceptance requires spec examples to load successfully under the new typed model. Parse-only checks can miss failures introduced in validation-time typed checks (ID wrappers, step/agent ID enforcement, output contract validation).
- Action: Update the test to call `orchestrator.validate(&settings, &orchestrator.id)` for each example fixture and assert success (or explicit migration guidance where intended).

2. **Low**: Integration ID-validation coverage is incomplete for `StepId` despite adding wrapper enforcement.
- Evidence: `src/config.rs:667` enforces `StepId::parse(&step.id)`, but integration tests only cover orchestrator/workflow/agent command validation (`tests/config_typing_foundations.rs:109` onward).
- Why this matters: P14-T02 asks for integration coverage of command/setup ID validation behavior across introduced wrappers. Step ID is currently only indirectly covered.
- Action: Add an integration test path that attempts invalid step IDs through user-facing flows (CLI step creation/edit and/or setup TUI step add/rename), and assert actionable error text.

3. **Low**: Setup still keeps a fallback manual identifier validator branch, creating drift risk from typed wrappers.
- Evidence: `src/cli/setup_tui.rs:186` routes known ID kinds through wrappers, but unknown kinds fall back to `is_valid_identifier` at `src/cli/setup_tui.rs:179` and `src/cli/setup_tui.rs:193`.
- Why this matters: Phase goal is to reduce stringly validation and centralize constraints in typed constructors. The fallback path can diverge silently from `config` rules over time.
- Action: Remove or constrain fallback behavior (e.g., return explicit unsupported-kind error), or route all setup ID validation through shared typed helpers in `config`.

## Verification Run
- Executed from `nix-shell`:
  - `cargo test --test config_typing_foundations --test cli_command_surface --test slack_channel_sync`
- Result: all tests passed.
