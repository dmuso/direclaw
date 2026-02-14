# Phase 17 Review: Type-Driven Setup TUI Navigation

## Findings (Action Required)

1. High: The reducer is still root-menu-only; sub-screen transitions/edit actions are not handled by typed navigation.
The phase-17 task and plan call for a central typed transition path and screen-by-screen migration, but `setup_transition(...)` only models concrete behavior for `SetupScreen::Root`. For all non-root screens it returns generic no-op feedback for most actions, while real behavior remains in imperative per-screen loops.
- Evidence:
  - Non-root actions are treated as generic no-op feedback (`src/cli/setup_tui.rs:1613`, `src/cli/setup_tui.rs:1627`).
  - Root loop delegates to legacy screen loops instead of typed per-screen transitions (`src/cli/setup_tui.rs:1725`, `src/cli/setup_tui.rs:1753`).
- Spec/task references:
  - Central reducer and typed screen migration expectations (`docs/build/config-typing-and-type-driven-setup-tui-plan.md:85`, `docs/build/config-typing-and-type-driven-setup-tui-plan.md:138`, `docs/build/config-typing-and-type-driven-setup-tui-plan.md:142`).
  - P17-T02 acceptance criterion for central transition handling (`docs/build/tasks/phase-17-type-driven-setup-tui-navigation.md:24`).
- Action:
  - Move sub-screen navigation/edit intents into typed `SetupAction`/`setup_transition(...)` paths screen-by-screen.
  - Keep render/input loops thin: dispatch typed actions, apply transition/effect, then render from typed view models.

2. Medium: P17-T03 required integration/regression coverage is missing while task status is marked complete.
The task requires end-to-end setup navigation/save-cancel integration tests plus regression coverage for supported hotkeys/behavior, but the uncommitted test additions are unit tests in `setup_tui.rs` only.
- Evidence:
  - New tests are reducer/key-mapping unit tests in `src/cli/setup_tui.rs` (`src/cli/setup_tui.rs:4084`, `src/cli/setup_tui.rs:4210`).
  - P17-T03 explicitly requires integration + regression tests (`docs/build/tasks/phase-17-type-driven-setup-tui-navigation.md:38`, `docs/build/tasks/phase-17-type-driven-setup-tui-navigation.md:40`).
- Action:
  - Add integration tests in `tests/` that drive `direclaw setup` navigation/save/cancel flows and verify behavior parity for previously supported hotkeys.
  - Keep task status at `in_progress` until these required tests exist and pass.
