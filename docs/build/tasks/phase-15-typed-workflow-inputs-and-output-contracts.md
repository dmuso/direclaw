# Phase 15: Typed Workflow Inputs and Output Contracts

## Goal

Replace unstructured workflow inputs and optional output contract fields with strongly typed, structurally required models.

## Tasks

### P15-T01 Replace `WorkflowConfig.inputs` YAML blob with typed workflow input model

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `inputs` uses a typed wrapper over validated input keys.
  - Input key normalization/validation is centralized in one place.
  - Transitional parsing supports legacy input representation if required.
- Automated Test Requirements:
  - Unit tests for input model parse/serialize round-trip.
  - Unit tests for invalid input key shapes/values.
  - Integration test confirming setup-edited inputs persist and reload correctly.

### P15-T02 Make step outputs/output_files required in in-memory config model

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `WorkflowStepConfig.outputs` is non-optional in typed model.
  - `WorkflowStepConfig.output_files` is non-optional in typed model.
  - It is impossible to construct an in-memory step without output contract fields.
- Automated Test Requirements:
  - Unit tests for model constructors and serde behavior.
  - Unit tests for output contract key parsing (required/optional marker semantics).

### P15-T03 Migrate validation/runtime/setup scaffolds to required contract semantics

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Validation logic no longer performs optional-presence checks for output contract fields.
  - Setup/CLI scaffold generation always emits contract-safe defaults.
  - Runtime output handling behavior remains unchanged for valid configs.
- Automated Test Requirements:
  - Integration tests for setup-generated workflow/step scaffolds.
  - Integration tests covering workflow run output mapping/file writes with typed contract.
