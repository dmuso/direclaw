# Workspace Access and Isolation

## Scope

Defines orchestrator workspace guarantees, shared workspace allowlists, step workspace modes, and pre-execution access validation.

## Orchestrator Private Workspace Rules

Every orchestrator always has a private workspace:

- Default: `<workspaces_path>/<orchestrator_id>`
- Optional override: `orchestrators.<orchestrator_id>.private_workspace`

Private workspace resolution is deterministic:

1. Use `orchestrators.<orchestrator_id>.private_workspace` when configured.
2. Else use `<workspaces_path>/<orchestrator_id>`.

This private workspace is always available to the orchestrator.

Per-orchestrator config and local assets live under this private workspace, including:

- `<orchestrator_private_workspace>/orchestrator.yaml`
- workflow definition folders referenced by that orchestrator config
- workflow run work areas under `<orchestrator_private_workspace>/work/runs/<run_id>`

Provider execution guarantee:

- Agents execute from the orchestrator private workspace root (process CWD is `<orchestrator_private_workspace>`).

## Shared Workspace Rules

Shared areas are logical names in `settings.shared_workspaces`.

Access model:

- Deny by default.
- Global settings assign shared access per orchestrator via `orchestrators.<orchestrator_id>.shared_access[]`.
- Shared workspace access is orchestrator-level only (no per-agent shared grants).
- Effective execution workspace context is:
  - orchestrator private workspace root
  - plus shared areas explicitly allowlisted for the orchestrator

Domain expert guidance:

- Subject-matter knowledge files should live in orchestrator private workspace or domain-specific shared areas.
- Default workflows mapped from channel profiles must use orchestrators whose `shared_access` grants match that domain only.

## Path Security and Validation

For shared area definitions:

- Path must be absolute.
- Path must be canonicalized.
- Missing/non-resolvable paths fail validation.

Execution guard:

- Workspace access checks must run before provider execution.
- Unauthorized path access attempts must be rejected and logged.

## Workflow Step Workspace Modes

`workflow.steps[].workspace_mode` supports only:

- `orchestrator_workspace`
- `run_workspace`

Run workspace location:

- `<orchestrator_private_workspace>/work/runs/<run_id>`

Validation rules:

- `agent_workspace` is invalid and must fail config validation.

## Orchestrator Workspace Configuration Behavior

`orchestrator add`:

- Creates private workspace for the new orchestrator using the resolved path.
- Defaults to zero shared-area grants.
- Bootstraps `<orchestrator_private_workspace>/orchestrator.yaml`.

Configuration commands must support:

- Grant shared-area access to an orchestrator
- Revoke shared-area access from an orchestrator
- Set/unset per-orchestrator private workspace override
- Display current private path and shared access list via `orchestrator show`

## Acceptance Criteria

- Orchestrator with no grants sees only private workspace.
- Orchestrator with grants sees private plus exact allowlisted shared paths.
- Misconfigured shared paths fail startup/config validation.
- Access checks prevent usage of ungranted shared areas.
- Agent execution CWD resolves to orchestrator private workspace root.
- Workflow `run_workspace` resolves under `<orchestrator_private_workspace>/work/runs/<run_id>`.
- Domain-specific channel-profile workflows do not gain cross-domain shared area access unless explicitly granted to the owning orchestrator.
