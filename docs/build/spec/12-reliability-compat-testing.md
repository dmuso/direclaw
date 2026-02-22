# Reliability, Compatibility, and Testing

## Scope

Defines non-functional reliability requirements, compatibility posture, and required test strategy.

DireClaw v1 channel scope is Slack-only.
Discord, Telegram, and WhatsApp compatibility and adapter testing are deferred after v1.

## Reliability Requirements

System must guarantee:

- Queue claims/file moves are atomic where possible.
- Processing failures do not silently drop messages.
- Channel-originated messages route through orchestrator selection + workflow dispatch.
- Per-agent ordering is preserved.
- Cross-agent concurrency is independent.
- Worker restarts are safe for partially processed queue files.
- Workspace access checks run before provider execution.
- Misconfigured shared paths fail fast.
- Legacy agent config fields (`private_workspace`, `shared_access`) fail fast.
- Agent execution CWD is orchestrator private workspace root.
- Workflow run task/work areas are under `<orchestrator_private_workspace>/work/runs/<run_id>`.
- Workflow loops/timeouts are enforced.
- Unauthorized workflow starts are rejected and logged.
- Active workflow runs publish observable progress snapshots.
- Slack workflow threads receive periodic orchestrator progress posts every 15 minutes until terminal run state.
- Diagnostics investigations use bounded retrieval, strict scope enforcement, and persisted audit artifacts.

## Compatibility Posture

Current beta releases do not support backward compatibility or migration from older config/queue layouts.

Requirements:

- Enforce the documented typed config shape at parse/validation time.
- Reject legacy/unknown config shapes with explicit validation errors.
- Reject removed `workflow.steps[].workspace_mode` value `agent_workspace` with explicit validation errors.
- Persist workflow-run records only at canonical paths.

## Test Strategy

### Unit Tests

- routing parser
- selector request builder and parser (including `action=workflow_start|workflow_status|diagnostics_investigate|command_invoke`)
- channel-profile -> orchestrator mapping validation
- per-orchestrator config loading and validation
- model mapping
- file-tag extraction and stripping
- message splitters
- config validation
- workspace access resolution and shared-area allowlist enforcement
- workflow step workspace mode validation (`orchestrator_workspace`/`run_workspace` only)
- legacy orchestrator-agent field rejection (`private_workspace`, `shared_access`)
- workflow schema and transition validation
- diagnostics scope resolver and ambiguity handling
- diagnostics retrieval ranking and hard limits
- diagnostics path allowlist enforcement

### Integration Tests

- queue lifecycle (`incoming -> processing -> outgoing`)
- channel inbound -> orchestrator selector -> workflow dispatch
- per-agent execution ordering
- reset flag behavior
- private plus shared workspace visibility behavior
- provider execution CWD remains orchestrator private workspace root
- workflow step run workspace resolves under `<orchestrator_private_workspace>/work/runs/<run_id>`
- workflow execution including approval/rejection loops and timeout handling
- per-conversation ordering with channel-profile context
- selector retry and default-workflow baseline behavior
- selector `command_invoke` routing executes only functions present in `availableFunctions` and rejects unknown function ids
- selector request/result file persistence and replayability
- heartbeat worker startup semantics respect `monitoring.heartbeat_interval` enable/disable behavior
- heartbeat ticks enqueue one message per configured agent across orchestrators and monitor outbound response correlation without mutating outbound artifacts
- heartbeat tick failures report degraded worker health and runtime logs without crashing supervisor
- status-check intent handling uses selector-agent inference for natural-language requests (for example `what's the latest update`) and returns run progress without advancing workflow steps
- diagnostics intent handling uses selector-agent inference for natural-language requests (for example `why did this fail`) and returns investigation findings without advancing workflow steps
- natural-language command intent handling routes through selector `command_invoke` for supported CLI-parity functions
- periodic active-run progress heartbeat updates `progress.json` at least every 15 minutes
- diagnostics context bundle persistence and replayability
- diagnostics loop limits enforce clarifying-question fallback when scope/evidence is insufficient

### Adapter Tests

- inbound event -> queue payload mapping
- slack channel profile identity mapping (`channelProfileId`)
- outbound delivery semantics
- file round-trip behavior
- `/agent` and `/reset` command handling
- workflow directives never leak to end-user channels
- Slack thread progress posts are emitted every 15 minutes for active workflow runs
- Slack thread diagnostics requests return natural-language findings and evidence summary
- post-v1 adapters (Discord/Telegram/WhatsApp) add equivalent adapter suites when promoted into release scope

### End-to-End Smoke Tests

- start daemon
- inject queue messages
- run workflow executions
- validate domain channel profile -> selector choice -> workflow -> expert agent path
- verify outbound responses and attachments
- issue diagnostics query and verify bounded evidence-based response with persisted audit files

## Delivery Milestones

1. Core settings and queue models
2. Queue processor and routing
3. Provider runners
4. Slack adapter and worker lifecycle integration (v1)
5. Agent/workspace commands and workflow definitions
6. Workflow orchestrator runtime
7. Heartbeat worker
8. Daemon operations
9. Installer/updater and release tooling
10. Hardening, workspace/workflow security, compatibility posture, and release readiness
11. Post-v1 channel adapters (Discord/Telegram/WhatsApp)

## Acceptance Criteria

- Reliability requirements are testable and mapped to automated test coverage.
- Compatibility posture is explicit and enforced by automated validation tests.
- Milestone sequence is executable without circular dependencies.
