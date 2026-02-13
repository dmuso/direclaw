# Phase 06: CLI and Daemon Operations

## Goal

Provide full command-surface management for orchestrators, workflows, channel profiles, and daemon lifecycle.

## Tasks

### P06-T01 Implement daemon lifecycle commands

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `start|stop|restart|status|logs|attach` behave per spec and are repeatable.
  - `status` reports real process state including per-channel-profile health for Slack.
  - `attach` returns orchestrator-generated workflow/process status summary when no attachable supervisor/session exists.
- Automated Test Requirements:
  - Unit tests for command argument and state validation.
  - Integration tests for lifecycle transitions and status/log output behavior.

### P06-T02 Implement orchestrator and orchestrator-agent management commands

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Orchestrator CRUD and private/shared workspace commands update config and bootstrap files.
  - Orchestrator-agent CRUD/reset commands update orchestrator config and workspace layout.
  - `orchestrator show` reports private path and shared access list.
- Automated Test Requirements:
  - Unit tests for command mutation logic and validation failures.
  - Integration tests for bootstrap/create/update/remove flows without manual YAML edits.

### P06-T03 Implement workflow and channel-profile command suites

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Workflow commands support list/show/run/status/progress/cancel with orchestrator scoping.
  - `workflow status` and `workflow progress` are read-only and do not mutate run state.
  - Channel-profile commands support CRUD and orchestrator mapping updates with validation.
- Automated Test Requirements:
  - Unit tests for scoping, authorization, and read-only enforcement.
  - Integration tests for full command lifecycle and orchestrator/channel mapping behavior.

### P06-T04 Expose selector-callable function registry for command parity

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Every required CLI/daemon command has a stable function registry entry.
  - Registry entries include function id, schema, and selector-facing description.
  - Selector `command_invoke` accepts only functions present in `availableFunctions`.
- Automated Test Requirements:
  - Unit tests for function registry generation and schema validation.
  - Integration test validating `command_invoke` allowlist and unknown-function rejection.
