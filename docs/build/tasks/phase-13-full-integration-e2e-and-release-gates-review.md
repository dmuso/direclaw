# Phase 13 Review: Actions Required

Reviewed artifacts:
- `docs/build/tasks/phase-13-full-integration-e2e-and-release-gates.md`
- `docs/build/workflow-system-implementation-plan.md`
- `tests/message_flow_queue_orchestrator_provider_e2e.rs`
- `tests/cli_command_surface.rs`
- `tests/runtime_supervisor.rs`

## Findings (ordered by severity)

1. **Medium**: Phase-13 E2E does not validate terminal outbound response semantics required by plan.
- Spec expectation: `docs/build/workflow-system-implementation-plan.md:282` requires "outbound response reflects terminal result" for channel-ingress E2E.
- Task acceptance/claim: `docs/build/tasks/phase-13-full-integration-e2e-and-release-gates.md:18` and `docs/build/tasks/phase-13-full-integration-e2e-and-release-gates.md:26` state channel-ingress to terminal response is covered.
- Current assertion only checks startup acknowledgement (`"workflow started"`) in `tests/message_flow_queue_orchestrator_provider_e2e.rs:279`.
- **Action**: Add/extend E2E assertions so the outbound queue payload proves terminal result semantics (or explicitly update the plan/task wording if runtime contract is async-ack-only by design).

2. **Low**: "No config-only fields remain unconsumed" is marked complete without direct CLI-path proof for orchestration timeout fields.
- Task claim: `docs/build/tasks/phase-13-full-integration-e2e-and-release-gates.md:52` and `docs/build/tasks/phase-13-full-integration-e2e-and-release-gates.md:59`.
- Test sets orchestration timeout fields (`default_run_timeout_seconds`, `default_step_timeout_seconds`, `max_step_timeout_seconds`) in `tests/cli_command_surface.rs:447`, but current assertions only prove success path, retries, and artifacts; they do not demonstrate those timeout settings are consumed/enforced through the CLI-configured runtime path.
- **Action**: Add a CLI integration assertion that fails/passes based on these timeout fields (for example, a deliberately slow provider step capped by orchestrator-level step timeout), or narrow the completion claim to fields already proven.

## Verification run

Executed successfully in `nix-shell`:
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all`
