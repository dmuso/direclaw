# Phase 14: Config Typing Foundations

## Goal

Replace high-risk stringly config fields with typed enums/newtypes while preserving YAML compatibility and spec behavior.

## Tasks

### P14-T01 Introduce typed enums for provider, channel kind, and workflow step type

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `provider` is represented by a typed enum in the config domain.
  - `channel` is represented by a typed enum in channel profiles.
  - workflow step `type` is represented by a typed enum.
  - YAML serialization remains compatible with existing snake_case values.
- Automated Test Requirements:
  - Unit tests for serde parse/serialize round-trip for each enum.
  - Unit tests for invalid enum values returning clear parse/validation errors.

### P14-T02 Add typed identifier wrappers for orchestrator/workflow/step/agent ids

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - New id wrappers enforce existing identifier constraints at construction/parse time.
  - Config and setup paths use wrappers for newly touched code paths.
  - User-facing error messages remain actionable when ids are invalid.
- Automated Test Requirements:
  - Unit tests for valid/invalid id creation.
  - Integration tests covering command/setup id validation behavior.

### P14-T03 Preserve compatibility and migrate internal callsites

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Existing spec example settings/orchestrator files load successfully, or produce explicit migration errors with guidance.
  - Existing CLI and setup flows continue operating with typed fields.
  - No new manual string normalization is required outside typed constructors.
- Automated Test Requirements:
  - Integration tests loading `docs/build/spec/examples/settings/*.yaml` and orchestrator examples.
  - Regression tests for setup bootstrap and save paths.
