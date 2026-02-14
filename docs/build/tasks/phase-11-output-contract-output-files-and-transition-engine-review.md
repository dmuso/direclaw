# Phase 11 Review: Output Contract, Output Files, and Transition Engine

## Scope Reviewed
- Uncommitted changes in:
  - `src/config.rs`
  - `src/orchestrator.rs`
  - `tests/orchestrator_workflow_engine.rs`
  - `docs/build/tasks/phase-11-output-contract-output-files-and-transition-engine.md`
- Compared against requirements in `docs/build/workflow-system-implementation-plan.md` (Phase 3/4 contract and fail-fast expectations).

## Findings (Action Required)

1. High: malformed `outputs` declarations are not rejected at config-validate time
- Evidence:
  - `src/config.rs:487` strips only a trailing `?` and checks emptiness/mapping, but does not reject embedded/invalid `?` usage (for example `plan?draft`).
  - Runtime parser explicitly rejects that form in `src/orchestrator.rs:1709` (`output key may only contain optional marker as trailing \`?\``).
- Why this is a problem:
  - This violates fail-fast config behavior: an invalid output contract can pass orchestrator config validation and only fail during step execution.
  - It creates a config/runtime validation mismatch and non-deterministic operational failure timing.
- Required action:
  - Reuse equivalent output-key parsing rules in config validation (same rule as `parse_output_contract_key`) so invalid declarations are rejected before runtime.
  - Add a config unit test that asserts orchestrator validation fails for malformed output keys containing non-trailing `?`.

## Verification Notes
- `nix-shell --run "cargo test --all"` passed.
- `nix-shell --run "cargo clippy --all-targets --all-features -- -D warnings"` passed.
