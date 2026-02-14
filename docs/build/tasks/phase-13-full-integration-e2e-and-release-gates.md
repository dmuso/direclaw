# Phase 13: Full Integration, E2E, and Release Gates for Workflow System

## Goal
Prove the workflow system is fully functional end-to-end and prevent regressions by enforcing comprehensive integration and E2E release gates.

## Dependencies
- `docs/build/spec/07-channel-adapters.md`
- `docs/build/spec/12-reliability-compat-testing.md`
- `docs/build/release-checklist.md`
- `docs/build/workflow-system-implementation-plan.md`

## Tasks

### P13-T01 Build complete workflow E2E suite (channel ingress to terminal run)

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - E2E test covers inbound channel message -> selector -> workflow start -> multi-step execution -> terminal response.
  - E2E validates run and progress artifacts at every major transition.
  - E2E validates output files exist with expected content and safe paths.
- Verification:
  - Add/expand tests in `tests/message_flow_queue_orchestrator_provider_e2e.rs`.
  - Add E2E fixture workflows including review loops and mapped outputs.
  - Run `nix-shell --run "cargo test --test message_flow_queue_orchestrator_provider_e2e"`.
- Implemented:
  - Added `channel_ingress_multi_step_workflow_reaches_terminal_state_and_writes_safe_outputs` with channel ingress -> selector -> multi-step execution (`agent_task` + `agent_review`) -> terminal success assertions.
  - Extended channel-ingress E2E to issue a workflow-bound `/status` follow-up and assert outbound terminal semantics include `state=succeeded`.
  - Added run/progress/engine-log transition assertions and per-step attempt artifact checks.
  - Added output-file path safety and content assertions under run-scoped outputs.

### P13-T02 Add full negative-path E2E coverage for workflow safety and resilience

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - E2E includes provider timeout/non-zero/parse failure scenarios.
  - E2E includes malicious `output_files` rejection and security log verification.
  - E2E includes restart recovery with in-flight processing file handling.
- Verification:
  - Add negative-path E2E fixtures and assertions for terminal failure reasons.
  - Add restart/recovery E2E scenario to runtime supervisor tests.
  - Run `nix-shell --run "cargo test --test message_flow_queue_orchestrator_provider_e2e --test runtime_supervisor"`.
- Implemented:
  - Added `malicious_output_file_template_is_rejected_and_security_log_records_denial` in `tests/message_flow_queue_orchestrator_provider_e2e.rs`.
  - Added `start_recovers_processing_entry_and_processes_recovered_message` in `tests/runtime_supervisor.rs`.
  - Provider timeout/non-zero/parse failure E2E scenarios remain covered by existing tests in `tests/message_flow_queue_orchestrator_provider_e2e.rs`.

### P13-T03 Validate CLI and setup TUI parity against executable engine behavior

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `workflow run/list/show/status/progress/cancel` behavior matches engine-backed functionality.
  - Setup TUI workflow fields (`inputs`, step fields, outputs/output_files, orchestration limits) round-trip to runtime behavior.
  - No config-only fields remain unconsumed by execution path.
- Verification:
  - Expand CLI command surface tests for workflow runtime assertions.
  - Add config round-trip integration test from TUI-edited workflow definitions.
  - Run `nix-shell --run "cargo test --test cli_command_surface --test orchestrator_workflow_engine"`.
- Implemented:
  - Added `workflow_runtime_consumes_tui_style_fields_end_to_end` in `tests/cli_command_surface.rs`.
  - Added `workflow_run_enforces_orchestration_timeouts_from_cli_config` in `tests/cli_command_surface.rs`.
  - Tests cover `workflow list/show/run/status/progress/cancel` and verify runtime consumption of TUI-style fields (`inputs`, step `outputs/output_files`, step retries, workflow/orchestrator limits/timeout enforcement, and transition wiring).

### P13-T04 Enforce workflow subsystem release gates

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - CI gate includes full workflow unit/integration/E2E suites.
  - Any failing workflow safety/reliability test blocks merge.
  - Release readiness evidence includes passing workflow matrix and artifact checks.
- Verification:
  - Execute full gate sequence:
    - `nix-shell --run "cargo fmt --all"`
    - `nix-shell --run "cargo clippy --all-targets --all-features -- -D warnings"`
    - `nix-shell --run "cargo test --all"`
  - Record gate evidence in review/report docs.
- Implemented:
  - CI/release workflows already enforce the full workflow matrix through `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all`.
  - Added Phase 13 evidence report in `docs/build/review/actioned/` documenting gate execution and workflow-focused suites.
