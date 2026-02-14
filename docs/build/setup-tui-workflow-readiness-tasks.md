# Setup TUI and Starter Workflow Readiness Tasks

This checklist captures the required work to ensure a fresh `direclaw setup` produces a complete, runnable workflow experience without manual YAML hardening.

## 1. Make Starter Templates Runtime-Executable

- Update `initial_orchestrator_config(...)` starter workflows so every step prompt explicitly instructs provider output in a single `[workflow_result] ... [/workflow_result]` JSON envelope.
- Ensure starter prompts include enough schema guidance for deterministic output keys.
- For steps that should persist artifacts, declare `outputs` and matching `output_files` mappings.
- For `agent_review` steps, require explicit `decision` output (`approve|reject`) in prompt contract.

## 2. Remove Placeholder Workflow/Step Scaffolds

- Replace `workflow add` default step prompt (`"placeholder"`) with a contract-safe scaffold.
- Replace setup TUI “Add Workflow” and “Add Step” default prompts with contract-safe scaffolds.
- Default scaffolds should include:
  - `[workflow_result]` JSON envelope instruction
  - a starter output contract
  - starter `output_files` mapping for declared outputs

## 3. Align Orchestrator Config Persistence With Spec

- Persist orchestrator config to canonical path:
  - `<resolved_orchestrator_private_workspace>/orchestrator.yaml`
- Keep global registry only as compatibility/migration support if needed.
- Ensure setup and command paths consistently read/write the canonical per-orchestrator file.

## 4. Implement `workspace_mode` End-to-End

- Add `workspace_mode` to workflow step config model and serialization.
- Add validation for supported modes:
  - `orchestrator_workspace` (default)
  - `run_workspace`
  - `agent_workspace`
- Update execution engine to honor mode when selecting working directory/cwd.
- Expose/edit `workspace_mode` in setup TUI workflow-step editor.

## 5. Make Fresh Setup Channel-Ready (Optional but Recommended)

- Add guided setup path for at least one channel profile mapping to the primary orchestrator.
- Validate minimal channel-profile completeness in setup flow.
- Ensure post-setup messaging path is operable without hidden manual steps.

## 6. Add Readiness Tests (Acceptance Gates)

- Add integration test: fresh `setup` then `workflow run main default` succeeds with contract-following provider mock.
- Add test to assert generated starter step prompts contain `[workflow_result]` contract instructions.
- Add test coverage for canonical orchestrator config file creation under orchestrator private workspace.
- Add tests for `workspace_mode` parsing, validation, and execution behavior.
- Add test coverage for setup-generated workflow/step scaffolds (CLI + TUI paths) to prevent regressions.
