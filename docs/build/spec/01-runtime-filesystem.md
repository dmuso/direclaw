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

Global control-plane structure (runtime-only):

- `~/.direclaw/config.yaml`
- `~/.direclaw/config-orchestrators.yaml`
- `~/.direclaw/daemon/runtime.json`
- `~/.direclaw/daemon/supervisor.lock`
- `~/.direclaw/logs/runtime.log`
- `~/.direclaw/runtime/preferences.yaml`
- `~/.direclaw/channels/*` (adapter-global runtime state)

Per-orchestrator execution structure:

- `<orchestrator_runtime_root>/queue/incoming`
- `<orchestrator_runtime_root>/queue/processing`
- `<orchestrator_runtime_root>/queue/outgoing`
- `<orchestrator_runtime_root>/files`
- `<orchestrator_runtime_root>/logs/orchestrator.log`
- `<orchestrator_runtime_root>/orchestrator/messages`
- `<orchestrator_runtime_root>/orchestrator/select/incoming`
- `<orchestrator_runtime_root>/orchestrator/select/processing`
- `<orchestrator_runtime_root>/orchestrator/select/results`
- `<orchestrator_runtime_root>/orchestrator/select/logs`
- `<orchestrator_runtime_root>/orchestrator/diagnostics/incoming`
- `<orchestrator_runtime_root>/orchestrator/diagnostics/processing`
- `<orchestrator_runtime_root>/orchestrator/diagnostics/context`
- `<orchestrator_runtime_root>/orchestrator/diagnostics/results`
- `<orchestrator_runtime_root>/orchestrator/diagnostics/logs`
- `<orchestrator_runtime_root>/workflows/runs`
- `<orchestrator_runtime_root>/work/runs/<run_id>`

`<orchestrator_runtime_root>` resolves to the orchestrator private workspace root.

Agent execution workspace model:

- Provider-backed agent processes execute with current working directory set to `<orchestrator_runtime_root>`.
- Workflow run task/work artifacts are created under `<orchestrator_runtime_root>/work/runs/<run_id>`.

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

Shared workspace areas are logical names mapped to metadata objects:

- Example logical names: `shared`, `docs`, `data`
- Registry source: global config field `shared_workspaces`
- Entry shape:
  - `path` (absolute path)
  - `description` (required non-empty text describing intended usage for agents/operators)

Rules:

- Shared areas are deny-by-default.
- Each orchestrator gets shared-area grants in global config:
  - `orchestrators.<orchestrator_id>.shared_access[]`
- Shared workspace access is orchestrator-scoped only.
- Shared paths must be absolute and canonicalized.
- Missing or invalid shared paths must fail validation with explicit errors.

## Acceptance Criteria

- All required directories are created at setup/start time if absent.
- Startup fails fast on invalid configured shared workspace paths.
- Each enabled runtime worker can be started and observed as independent process state.
- Every channel-originated message execution path flows through orchestrator-owned workflow selection and workflow dispatch.
