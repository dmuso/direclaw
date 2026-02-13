# Heartbeat Automation Service

## Scope

Defines periodic agent heartbeat message generation and response logging.

## Scheduling

- Run interval: `monitoring.heartbeat_interval` seconds
- Default interval: `3600`

## Per-Agent Execution Rules

For each configured agent:

1. Load `<agent_dir>/heartbeat.md`.
2. If missing, use default short fallback prompt.
3. Enqueue heartbeat message targeting that specific agent.

Post-enqueue behavior:

- Inspect outbound queue for matching heartbeat responses.
- Log response snippets for monitoring visibility.

## Integration Rules

- Heartbeat messages use same queue contract and processing guarantees as other messages.
- Heartbeat outbound queue naming follows queue spec heartbeat naming rule.
- Heartbeat worker may be disabled; if disabled, no scheduled enqueue occurs.

## Acceptance Criteria

- Heartbeat messages are generated for every configured agent on schedule.
- Missing `heartbeat.md` does not block heartbeat execution.
- Matching outbound responses are observable in logs.

