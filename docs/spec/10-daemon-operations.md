# Daemon Lifecycle and Operations

## Scope

Defines required top-level operational commands and runtime management behavior.

## Required Command Set

- `start`
- `stop`
- `restart`
- `status`
- `logs`
- `setup`
- `send`
- `channels reset`
- `provider`
- `model`
- `agent`
- `workflow`
- `update`
- `attach`

Natural-language chat parity:

- Every command above must have a selector-callable function form in orchestrator `availableFunctions`.
- Channel-originated natural-language requests may be routed by selector to `command_invoke` action for these commands when argument requirements are satisfiable.

## Operational Semantics

`start`:

- Initialize runtime environment.
- Launch enabled workers (channels, queue processor, orchestrator, optional heartbeat).
- For Slack, initialize all configured channel profiles and expose per-profile readiness state.

`stop`:

- Gracefully stop running workers and release runtime handles.

`restart`:

- Equivalent to stop then start with same effective config.

`status`:

- Report worker health and process state.
- Include per-channel and per-channel-profile health (especially Slack channel profiles).

`logs`:

- Tail structured logs from state log directory.

`update`:

- Support release checks.
- Support in-place updates with backup/rollback safety.

`attach`:

- Attach to running process supervisor/session where applicable.
- If no attachable supervisor/session exists, run orchestrator inspection and return current workflow/process status summary instead of failing.

## Acceptance Criteria

- Each command is implemented and stable under repeated use.
- Status output reflects actual running workers.
- Update flow can roll back on failed update transitions.
