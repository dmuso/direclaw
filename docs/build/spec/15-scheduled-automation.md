# Scheduled Automation and Cron Handling

## Scope

Defines recurring and one-shot scheduled automation, including natural-language schedule intent handling, persistent job management, trigger evaluation, and orchestrator/workflow dispatch semantics.

This spec covers scheduler domain behavior and orchestrator routing ownership.

## Goals

- Support user intent such as: "post hello world in #team every Friday at 9AM".
- Preserve orchestrator-first routing and queue lifecycle guarantees.
- Keep scheduling deterministic, auditable, and restart-safe.
- Keep channel target abstraction cross-channel, with adapter-specific validation delegated to channel specs.

## Scheduling Model

Supported schedule types:

- `once`: execute one time at an absolute instant.
- `interval`: execute every N seconds/minutes/hours.
- `cron`: execute using cron expression + timezone.

Cron v1 requirements:

- 5-field expression format: `minute hour day_of_month month day_of_week`.
- Optional named day/month aliases are allowed (`mon`, `fri`, `jan`, etc.).
- Timezone is required for cron schedules and must be an IANA timezone id (for example `America/Los_Angeles`).
- Trigger cadence resolution is minute-level.

## Job Data Model

Each scheduled job must persist:

- `jobId` (stable id)
- `orchestratorId`
- `createdBy` metadata (`channelProfileId`, `senderId`, `sender`, request message id when available)
- `schedule` (`once|interval|cron` plus type-specific fields)
- `targetAction`
- `targetRef` (optional generic target descriptor)
- `state` (`enabled|paused|disabled|deleted`)
- `misfirePolicy` (`fire_once_on_recovery|skip_missed`)
- `nextRunAt`
- `lastRunAt`
- `lastResult` summary
- `createdAt`, `updatedAt`

Persistence locations (under resolved orchestrator runtime root):

- Active jobs: `<orchestrator_runtime_root>/automation/jobs/<jobId>.json`
- Run history: `<orchestrator_runtime_root>/automation/runs/<jobId>/<runTs>.json`
- Optional scheduler cursor/checkpoint: `<orchestrator_runtime_root>/automation/scheduler_state.json`

`<orchestrator_runtime_root>` resolves to the orchestrator private workspace root.

## Target Action Model

v1 supports the following scheduler-executed actions:

- `workflow_start`
- `command_invoke`

`workflow_start` payload:

- `workflowId` (required)
- `inputs` object (optional)

`command_invoke` payload:

- `functionId` (required, must be in selector-available function set)
- `functionArgs` object (optional)

Recommendation:

- Prefer `workflow_start` for durable business behavior.
- Use `command_invoke` for bounded operational commands.

## Natural-Language Handling

Natural-language schedule intent must route through orchestrator selector handling.

Selector behavior requirements:

- Detect scheduling intent (create, list, update, pause, resume, delete, run-now).
- Normalize natural language into one explicit scheduler command invocation.
- Resolve ambiguity with deterministic clarification prompts when required fields are missing.
- Never bypass function validation; selector output must conform to `availableFunctions` contract.

Required selector-callable scheduler functions:

- `schedule.create`
- `schedule.list`
- `schedule.show`
- `schedule.update`
- `schedule.pause`
- `schedule.resume`
- `schedule.delete`
- `schedule.run_now`

Required minimum args:

- `schedule.create`: `orchestratorId`, `scheduleType`, schedule payload, `targetAction` payload
- `schedule.update`: `jobId` + patch fields
- `schedule.pause|resume|delete|show|run_now`: `jobId`

## Orchestrator Mapping Rules

### Channel-Originated Requests

For channel-originated natural-language scheduling requests:

1. Require `channelProfileId` in inbound queue payload.
2. Resolve `orchestratorId` from `channel_profiles.<channelProfileId>.orchestrator_id`.
3. Execute selector and scheduler command in that orchestrator scope.
4. Persist job under that orchestrator runtime root.

### CLI-Originated Requests

For CLI scheduling requests:

- `orchestratorId` is required unless command is explicitly profile-scoped.
- Validation must fail fast on unknown orchestrator.

### Scheduled Trigger Dispatch

When a job fires:

- Scheduler must dispatch execution in the job's persisted `orchestratorId` scope.
- Scheduler must not infer orchestrator from current runtime defaults.

## Queue and Execution Integration

Scheduled triggers must use standard queue/orchestrator routing paths.

Dispatch rules:

- Scheduler writes trigger payload to the resolved orchestrator incoming queue.
- Trigger payload must preserve deterministic correlation fields (`jobId`, trigger timestamp, execution id).
- Queue lifecycle remains canonical: `incoming -> processing -> outgoing`.
- Workflow and command execution must occur through existing orchestrator routing and validation paths.

Prohibited behavior:

- Direct provider CLI invocation from scheduler bypassing orchestrator routing.
- Direct adapter API calls from scheduler bypassing queue/orchestrator path.

## Time and Reliability Semantics

Scheduler evaluation requirements:

- Evaluate due jobs at least once per minute.
- Compute `nextRunAt` deterministically after each trigger.
- Persist run outcome and update job metadata atomically where possible.

Restart and misfire handling:

- On startup, load all enabled jobs and recover scheduler state.
- Apply `misfirePolicy` for missed windows while down.
- Default misfire policy: `fire_once_on_recovery`.

Idempotency and overlap protection:

- A single job must not dispatch overlapping trigger instances unless explicitly configured.
- At-least-once trigger dispatch is acceptable; duplicate prevention must use execution id tracking in scheduler run history.

## Target Abstraction Boundary

Scheduler target model is channel-agnostic:

- `targetRef` is selected-action metadata.
- For Slack-targeted actions, `targetRef` must use this typed contract:
  - `channel = "slack"`
  - `channelProfileId` (required)
  - `channelId` (required)
  - `threadTs` (required when `postingMode=thread_reply`)
  - `postingMode` (`channel_post|thread_reply`)
- Scheduler validates Slack `targetRef` required fields and type shape, but not transport reachability.

Channel-specific enforcement belongs in adapter specs:

- Slack channel/thread id resolution, permission checks, and posting constraints remain defined in `07-channel-adapters.md`.
- Scheduler spec must reference adapter behavior but not duplicate adapter transport rules.

## Security and Validation

- Enforce orchestrator workspace and shared-access policy before scheduled execution.
- Reject path traversal or untrusted file references in scheduler payloads.
- Validate cron expressions and timezone ids at create/update time.
- Reject schedules with invalid or out-of-bounds intervals.
- Audit-log all scheduler create/update/delete/pause/resume/trigger events.

## Observability

Required logs/events:

- scheduler started/stopped
- job created/updated/paused/resumed/deleted
- trigger due/trigger dispatched/trigger failed
- misfire recovery actions

Status surfaces:

- `schedule.list` and `schedule.show` expose `nextRunAt`, `lastRunAt`, `lastResult`, `state`.
- runtime status may include scheduler worker health and last heartbeat.

## Acceptance Criteria

- Natural-language scheduling intents can be resolved into valid scheduler function invocations.
- Cron, interval, and once schedules persist and trigger deterministically.
- Trigger dispatch executes in the correct orchestrator scope every time.
- Scheduled execution uses canonical queue + orchestrator routing paths.
- Scheduler state survives daemon restarts with correct misfire behavior.
- Channel target handling remains adapter-owned and is not duplicated in scheduler core.
