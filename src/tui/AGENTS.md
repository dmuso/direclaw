# AGENTS: `src/tui` Config-Projection Rules

## Scope
This file governs TUI implementation under `src/tui` (currently `src/tui/setup.rs`).

## Source Of Truth
`crate::config` types are the canonical schema.
- `Settings`, `OrchestratorConfig`, `WorkflowConfig`, `WorkflowStepConfig`, `AgentConfig`, and related enums/newtypes define valid structure and values.
- YAML serialization/deserialization behavior is owned by these config/domain types.
- TUI must not invent a second schema in parallel.

## Architectural Requirement
The TUI is a projection/editor over typed config state.
1. Load typed config from disk into domain structs.
2. Build view-model rows/screens from typed fields (projection).
3. Apply user edits through typed update operations.
4. Validate using domain validation methods.
5. Serialize typed config back to disk.

No direct string-map mutation should bypass typed config operations.

## Allowed UI State
Transient UI state is allowed only for interaction concerns:
- selection index
- focused screen
- status/hint messages
- in-progress text buffer

Transient UI state must not become an alternate config model. If a draft state exists, it must remain structurally aligned with config-domain types.

## Enum And Option Handling
- Enum-like fields must use typed option providers derived from domain types.
- Do not hardcode enum choices in key handlers.
- Option rendering may convert typed variants to display strings, but selected values must round-trip back to typed values.

## Field Editing Contract
Each editable field should be defined by typed metadata:
- field id
- label
- value accessor from typed config state
- editor kind (`Text`, `Toggle`, `Select`, etc.)
- apply function that updates typed config state

This metadata is the driver for rendering and edit dispatch.

## Validation Contract
- Validation must happen at domain boundaries (`validate(...)`, typed parsers/newtypes).
- TUI should present only valid options for constrained fields.
- Errors must come from typed validators/parsers, not duplicated ad-hoc UI rules where avoidable.

## Prohibitions
- No duplicated hand-maintained schema that can drift from `crate::config`.
- No free-text editing for finite enum domains.
- No screen-specific hardcoded option sets when they can be derived from typed metadata/options.
- No persistence path that writes unvalidated raw structures.

## Reflection Note
Rust has limited runtime reflection. In this codebase, “reflection” means explicit typed metadata/descriptor tables and variant providers generated from or aligned to domain types.

## Testing Expectations
- Unit tests must confirm TUI option providers match domain-supported variants.
- Tests must verify edits apply through typed update paths and persist correctly.
- Regression tests must catch drift between TUI field metadata and config-domain schema.
