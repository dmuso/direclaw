# Heartbeat Automation Service

## Scope

Defines periodic agent heartbeat message generation and response logging.

## Scheduling

- Run interval: `monitoring.heartbeat_interval` seconds
- Default interval: `3600`

## Per-Agent Execution Rules

For each configured agent:

1. Load `<agent_dir>/heartbeat.md`.
2. If missing, skip enqueue for that agent and log `heartbeat.prompt.missing`.
3. Enqueue heartbeat message targeting that specific agent.
4. Heartbeat payload ids/correlation metadata must be deterministic per orchestrator-agent tick (`messageId`, `conversationId`, `workflowRunId`).

Post-enqueue behavior:

- Inspect outbound queue for matching heartbeat responses.
- Log response snippets for monitoring visibility.
- Outbound inspection must be read-only; never consume/mutate outbound files needed by channel adapters.

## Integration Rules

- Heartbeat messages use same queue contract and processing guarantees as other messages.
- Heartbeat outbound queue naming follows queue spec heartbeat naming rule.
- Heartbeat worker may be disabled; if disabled, no scheduled enqueue occurs.

## Acceptance Criteria

- Heartbeat messages are generated on schedule for agents that define `<agent_dir>/heartbeat.md`.
- Missing `heartbeat.md` does not block heartbeat execution and emits `heartbeat.prompt.missing`.
- Matching outbound responses are observable in logs.
