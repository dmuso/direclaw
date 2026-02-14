# Config Typing and Type-Driven Setup TUI Plan

## Objective
Refactor DireClaw configuration and setup flows so config semantics are represented by strong Rust types end-to-end, and setup TUI navigation/editing is driven by typed screen/action models rather than ad-hoc string/state mutation.

This plan covers the four original changes discussed plus a fifth change for type-driven TUI navigation.

## Change Set

### 1. Replace Stringly Config Fields With Typed Enums/Newtypes
Scope:
- Replace free-form strings for high-signal config fields with typed enums/newtypes.
- Keep YAML compatibility with `serde` rename/alias support.

Target examples:
- `ChannelProfile.channel` -> `ChannelKind` enum (`local`, `slack`, and any currently supported values).
- `AgentConfig.provider` -> `ProviderKind` enum (`anthropic`, `openai`).
- `WorkflowStepConfig.step_type` -> `WorkflowStepType` enum (`agent_task`, `agent_review`).
- Introduce id wrappers where practical (`OrchestratorId`, `WorkflowId`, `StepId`, `AgentId`) with constructor validation.

Implementation notes:
- Keep serialized YAML shape stable unless a migration is explicitly required.
- Add `TryFrom<String>`/custom deserializers only when compatibility needs them.

Acceptance:
- Parsing invalid enum values fails with clear config errors.
- Existing spec examples still parse (or fail with explicit migration guidance).

### 2. Replace Unstructured Workflow Inputs With a Typed Model
Scope:
- Replace `WorkflowConfig.inputs: serde_yaml::Value` with a typed representation.

Target model:
- `WorkflowInputs` wrapper around a validated sequence of input keys.
- Optional support for legacy shapes through transitional deserializer logic.

Implementation notes:
- Centralize input-key normalization and validation.
- Remove TUI CSV<->YAML-value conversion logic once typed model is in place.

Acceptance:
- Inputs round-trip serialization is deterministic.
- Invalid input key shapes are rejected in config validation.

### 3. Make Step Output Contract Structurally Required
Scope:
- Model output contract as required data, not optional fields that are later required by validation.

Target model changes:
- `WorkflowStepConfig.outputs: Vec<OutputKey>` (non-optional).
- `WorkflowStepConfig.output_files: BTreeMap<OutputKey, PathTemplate>` (non-optional).

Implementation notes:
- Add migration-safe loader behavior if needed for legacy configs.
- Keep output key optional-marker semantics (`?`) explicit in typed parser/constructor.

Acceptance:
- Config can no longer represent steps missing contract fields.
- Validation logic shifts from "presence checks" to semantic checks.

### 4. Move Setup Editing to Typed Config Operations
Scope:
- Refactor setup codepaths so edits are applied through typed config operations rather than direct string mutation and repeated tree lookups.

Target shape:
- Introduce typed update helpers for orchestrator/workflow/step/agent edits.
- Consolidate duplicated update logic in setup menus.

Implementation notes:
- Keep command behavior/spec parity while changing internals.
- Preserve current setup defaults and starter templates.

Acceptance:
- Setup can edit all currently supported fields using typed update functions.
- Setup save path persists fully validated typed configs.

### 5. Make Setup TUI Navigation Type-Driven
Scope:
- Replace large imperative menu branching with typed navigation state and action handling.

Target architecture:
- `enum SetupScreen { ... }` for all screens/routes.
- `enum SetupAction { MoveUp, MoveDown, Enter, Back, Save, Cancel, Edit(FieldId), Add(EntityKind), Delete(EntityKind), ... }`.
- `NavState` for selection, focused screen, and transient status.
- Reducer-style transition function: `update(state, nav, action) -> UiFeedback`.
- Screen view-model builders that map typed state -> render rows.

Implementation notes:
- Keep rendering layer mostly passive (draw from view model).
- Ensure each screen defines typed editable fields and command hints.

Acceptance:
- Navigation transitions are unit-testable without terminal I/O.
- Invalid transitions are impossible or return typed errors.
- Existing setup UX behavior is preserved unless intentionally changed.

## Phased Delivery Plan

### Phase 0: Baseline and Safety Net
- Inventory current config parsing/validation and setup TUI flows.
- Add/expand tests that lock in current behavior for key setup flows before refactor.

DoD:
- Baseline tests cover setup bootstrap, orchestrator/workflow/step edits, and save path.

### Phase 1: Core Typed Config Primitives (Changes 1 and partial 2)
- Add enums/newtypes for channel/provider/step type and key identifiers.
- Wire serde + validation compatibility.
- Introduce typed workflow input wrapper with legacy compatibility path.

DoD:
- `cargo test --all` passes with new types in place.
- Spec example config fixtures parse successfully or emit deliberate migration errors.

### Phase 2: Required Output Contract Model (Change 3)
- Convert step output contract fields to required typed fields.
- Update starter/scaffold generation and loader defaults/migration behavior as needed.
- Simplify orchestration validation around outputs/output_files.

DoD:
- No step can exist in-memory without output contract fields.
- Runtime workflow tests continue passing with contract enforcement.

### Phase 3: Typed Setup Domain Operations (Change 4)
- Introduce `SetupState`/domain operations layer.
- Migrate setup mutations to typed operations incrementally by section:
  1. Orchestrators
  2. Workflows
  3. Steps
  4. Agents

DoD:
- Setup path no longer mutates deep structures directly from each key handler.
- Error messages remain clear and user-facing.

### Phase 4: Type-Driven TUI Navigation (Change 5)
- Introduce typed screen/action/nav model and reducer.
- Migrate screen by screen while preserving terminal UI rendering.
- Add transition table tests for major navigation paths.

DoD:
- Setup loop dispatches typed actions into typed transitions.
- Screen transitions and edit actions are comprehensively unit-tested.

### Phase 5: Hardening and Cleanup
- Remove dead compatibility helpers and duplicated parsers.
- Ensure docs/spec examples and user docs reflect final config model.
- Run full quality gates from `nix-shell`.

DoD:
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all`

## Testing Strategy

### Unit Tests
- Enum/newtype serde parse/serialize behavior.
- Input/output contract constructors and validators.
- Setup reducer transitions and invalid-action handling.

### Integration Tests
- `direclaw setup` bootstrap + save + reload round trip.
- Orchestrator/workflow/step edit flows persist correctly.
- Workflow runtime consumes typed fields exactly as before.

### Regression Gates
- Spec example settings/orchestrator files remain valid against parser contract.
- CLI and setup TUI parity remains intact for managed fields.

## Risks and Mitigations

Risk: Breaking YAML compatibility for existing users.
Mitigation: Use serde aliases/transitional parsers; emit explicit migration errors with actionable messages.

Risk: Large `setup_tui.rs` migration causing regressions.
Mitigation: Migrate screen groups incrementally with transition tests per group.

Risk: Tight coupling between typed domain and rendering.
Mitigation: Keep rendering as pure projection of typed view models.

Risk: Refactor churn across runtime, CLI, and tests.
Mitigation: Land in phases with stable compile/test gates after each phase.

## Milestone Acceptance Criteria
- Config domain model is strongly typed for channel/provider/step/input/output contract semantics.
- Setup state edits and navigation are type-driven and testable as pure transitions.
- Runtime/CLI behavior remains spec-aligned with no regression in queue/orchestrator/workflow paths.
- Full repo quality gates pass from `nix-shell`.
