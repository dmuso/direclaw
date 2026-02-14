# Phase 17: Type-Driven Setup TUI Navigation

## Goal

Implement typed screen/action navigation for setup TUI so transition logic is explicit, deterministic, and unit-testable.

## Tasks

### P17-T01 Define typed setup navigation model

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `SetupScreen` enum covers root and all setup sub-screens.
  - `SetupAction` enum covers movement, enter/back, save/cancel, edit/add/delete intents.
  - `NavState` tracks focused screen, selection index, and status/hint text.
- Automated Test Requirements:
  - Unit tests for screen/action enum mappings from key events.
  - Unit tests for nav-state initialization and selection boundary behavior.

### P17-T02 Implement reducer-style transition function and view-model projection

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - A central transition function handles screen/action updates.
  - Invalid transitions return typed errors or no-op feedback, not panics.
  - Rendering consumes typed view models from state instead of bespoke traversal logic.
- Automated Test Requirements:
  - Unit tests for transition table across major paths.
  - Unit tests for invalid transitions and edge cases (empty lists, deleted selections).

### P17-T03 Migrate setup TUI loop to typed navigation dispatch

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Main setup TUI event loop dispatches typed actions into typed transitions.
  - Existing interaction behavior (save/cancel, section navigation, edit entry) is preserved.
  - Terminal rendering remains stable on first-time and existing setup modes.
- Automated Test Requirements:
  - Integration tests for end-to-end setup navigation and save/cancel flows.
  - Regression tests for previously supported hotkeys and behavior.
