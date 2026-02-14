# Runtime and Filesystem Model

## Scope

Defines required long-lived processes, polling cadence, state root layout, and workspace roots.

## Required Runtime Processes

DireClaw must run these components as independent long-lived workers:

- One process per enabled channel adapter
- Queue processor
- Workflow orchestrator (required central dispatch path)
- Optional heartbeat worker

Orchestrator deployment modes:

- Supported modes: `standalone` or `integrated`
- Default mode: `standalone`
- Channel-originated messages must always be dispatched through orchestrator regardless of deployment mode.

Polling behavior:

- Queue polling interval: `1s`
- Channel outbound polling interval: `1s`

Supervisor behavior:

- Process supervision must support native mode and tmux-compatibility mode.
- tmux mode is mandatory for per-agent provider sessions (for example `claude`, `codex`, and related provider CLIs).

## State Root and Required Paths

Default state root: `~/.direclaw`

Required filesystem structure:

- `~/.direclaw/queue/incoming`
- `~/.direclaw/queue/processing`
- `~/.direclaw/queue/outgoing`
- `~/.direclaw/files`
- `~/.direclaw/logs/*.log`
- `~/.direclaw/config.yaml`
- `~/.direclaw/config-orchestrators.yaml`
- `~/.direclaw/orchestrator/messages`
- `~/.direclaw/orchestrator/select/incoming`
- `~/.direclaw/orchestrator/select/processing`
- `~/.direclaw/orchestrator/select/results`
- `~/.direclaw/orchestrator/select/logs`
- `~/.direclaw/orchestrator/diagnostics/incoming`
- `~/.direclaw/orchestrator/diagnostics/processing`
- `~/.direclaw/orchestrator/diagnostics/context`
- `~/.direclaw/orchestrator/diagnostics/results`
- `~/.direclaw/orchestrator/diagnostics/logs`
- `~/.direclaw/workflows/runs`

Configuration layering model:

- Global config: `~/.direclaw/config.yaml`
- Orchestrator registry: `~/.direclaw/config-orchestrators.yaml`
- Per-orchestrator config: `<orchestrator_private_workspace>/orchestrator.yaml`

## Workspace Roots

Config field:

- `workspaces_path` (default `~/.direclaw/workspaces`)

Private workspace root per orchestrator:

- Default: `<workspaces_path>/<orchestrator_id>`
- Optional override: `orchestrators.<orchestrator_id>.private_workspace`

Resolution rule:

1. If `orchestrators.<orchestrator_id>.private_workspace` is set, use it.
2. Otherwise use `<workspaces_path>/<orchestrator_id>`.

Validation:

- Resolved private workspace path must be absolute and canonicalizable.
- Invalid private workspace definitions must fail config validation.
- Path must exist or be created during orchestrator provisioning.
- `orchestrator.yaml` must exist under resolved private workspace.

## Shared Workspace Registry

Shared workspace areas are logical names mapped to absolute paths:

- Example logical names: `shared`, `docs`, `data`
- Registry source: global config field `shared_workspaces`

Rules:

- Shared areas are deny-by-default.
- Each orchestrator gets shared-area grants in global config:
  - `orchestrators.<orchestrator_id>.shared_access[]`
- Agents declared inside an orchestrator can only use shared areas granted to their orchestrator.
- Shared paths must be absolute and canonicalized.
- Missing or invalid shared paths must fail validation with explicit errors.

## Acceptance Criteria

- All required directories are created at setup/start time if absent.
- Startup fails fast on invalid configured shared workspace paths.
- Each enabled runtime worker can be started and observed as independent process state.
- Every channel-originated message execution path flows through orchestrator-owned workflow selection and workflow dispatch.
