# DireClaw v1 Release Readiness Plan

## Summary

This plan defines the work required to ship DireClaw as a fully working, end-to-end application that can be distributed to end users with high operational quality.

Release posture for v1:

- Quality-first release gating.
- No migration/rollback requirement for legacy users in v1 (no existing user base), but upgrade safety for v1+ is designed in.
- Binary distribution via GitHub Releases for:
  - macOS arm64
  - Linux x86_64
  - Linux arm64
- Slack is the supported channel adapter for v1. Other adapters are explicitly deferred and documented as out-of-scope.

## Current-State Gaps

1. Runtime lifecycle is mostly simulated.
- `start/stop/status` track state files, but no long-lived supervised worker system runs continuously.

2. Production queue orchestration loop is incomplete.
- Queue claiming, orchestrator routing, and provider execution exist as components, but are not wired into an always-on runtime path.

3. Slack is command-invoked instead of lifecycle-managed.
- `channels slack sync` exists, but not an autonomous runtime worker that runs while daemon is active.

4. Update flow is placeholder behavior.
- `update check/apply` currently returns static/skipped responses.

5. Release/distribution automation is missing.
- No `.github/workflows` for CI and release artifacts.

6. Spec/doc path consistency is broken.
- References still use a legacy spec path, while the canonical path is `docs/build/spec/...`.

7. Selector function parity is incomplete.
- Function registry does not expose full command parity required by spec.

8. Distribution project hygiene is incomplete.
- Missing standard release docs and policy files (`LICENSE`, `CHANGELOG`, `SECURITY`, `CONTRIBUTING`).

## v1 Scope Decisions

### In Scope

- Real daemon/supervisor behavior with long-running workers.
- End-to-end Slack flow: inbound -> queue -> orchestrator -> provider -> outbound.
- Full command surface with stable behavior.
- High-confidence automated testing including E2E and restart recovery.
- GitHub Releases with checksums and reproducible build pipeline.
- User and operator documentation sufficient for first-time install and production use.

### Out of Scope for v1

- Legacy migration tooling (`direclaw migrate`) for pre-v1 users.
- Multi-channel support beyond Slack (Discord/Telegram/WhatsApp deferred).
- In-place binary self-update if not implemented safely; if not implemented, it must hard-fail with clear guidance.

## Architecture and Interface Changes

1. Daemon lifecycle semantics
- Preserve command names: `start|stop|restart|status|logs|attach`.
- Implement actual worker process/thread lifecycle and health reporting.

2. Queue processor integration
- Continuous queue worker loop processes `incoming -> processing -> outgoing`.
- Crash-safe recovery for stale/partial `processing` state at startup.

3. Orchestrator execution wiring
- Route channel-originated messages through orchestrator path.
- Execute provider-backed selector and workflow paths in runtime loop.

4. Slack worker integration
- Manage Slack worker under daemon lifecycle when enabled.
- Expose profile-level readiness/health in `status`.

5. Selector function registry
- Expand function IDs and typed argument schema coverage for supported v1 commands.

6. Release/update contract
- `update check` integrates with GitHub release metadata.
- `update apply` must be either robustly implemented or explicitly unsupported (no fake success responses).

## Delivery Phases

### Phase 00: Scope Lock and Documentation Baseline

- Lock v1 feature scope and explicitly defer non-v1 channels.
- Resolve spec/doc path mismatch with canonical `docs/build/spec` usage.
- Establish requirement traceability from plan -> tasks -> tests.

### Phase 01: Runtime Supervisor and Worker Lifecycle

- Implement long-lived supervisor and worker orchestration.
- Ensure graceful start/stop/restart semantics and health reporting.
- Persist runtime state for observability without pretending workers are running.

### Phase 02: End-to-End Queue and Orchestrator Execution

- Wire queue claiming and dispatch into runtime worker loop.
- Execute orchestrator routing and provider invocations in production flow.
- Enforce crash recovery and no-drop queue guarantees.

### Phase 03: Slack Runtime Worker and Conversation Flow

- Convert Slack sync from command-only operation into worker-managed behavior.
- Preserve thread/conversation semantics and profile mapping.
- Ensure outbound delivery and error handling are operationally clear.

### Phase 04: Command/Selector Parity and UX Quality

- Complete selector function registry parity for supported v1 commands.
- Strengthen CLI output consistency and operational error messages.
- Add readiness checks (`doctor` style command) for install/runtime diagnostics.

### Phase 05: GitHub Release Automation and Distribution

- Add CI and release workflows.
- Build, package, and publish binaries for target matrix.
- Generate checksums and release notes; validate artifacts through smoke tests.

### Phase 06: Documentation, Hardening, and Release Gates

- Deliver end-user guide and operator runbook.
- Add/complete project hygiene files.
- Enforce release blocker checklist across tests/docs/artifacts.

## Automated Test Strategy

### Unit

- Config validation, selector parsing, workspace enforcement, queue transitions, provider invocation parsing.

### Integration

- Daemon lifecycle transitions, queue worker behavior, orchestrator dispatch, Slack adapter semantics.

### End-to-End

- Full path: message ingress to outbound response with persisted run/progress artifacts.
- Recovery path: restart handling with partial processing files.

### Release Validation

- CI checks: `fmt`, `clippy`, `test`, E2E suite, docs/link validation.
- Release artifact checks: build matrix completeness, checksum verification, binary smoke execution.

## Release Acceptance Criteria (Go/No-Go)

All conditions must be true before `v1.0.0` tag:

1. Runtime workers execute continuously under daemon lifecycle.
2. Slack end-to-end flow is automated and verified in tests.
3. Queue/orchestrator/provider pipeline runs in production path (not simulation-only behavior).
4. CI gates pass, including E2E and docs checks.
5. GitHub release workflow publishes expected binaries and checksums.
6. User docs and operator docs are complete and validated from clean environment install.
7. Placeholder or misleading operational responses are removed.
