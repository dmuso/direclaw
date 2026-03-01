# Configuration and Management Commands

## Scope

Defines required configuration surface and command interfaces for agent, workflow, provider, and model management.

Canonical config files:

- Global settings: `~/.direclaw/config.yaml`
- Orchestrator registry snapshot: `~/.direclaw/config-orchestrators.yaml`

## Required Configuration Domains

Setup/config must support:

- Channel enablement and credentials
- Channel profiles and channel-profile -> orchestrator mapping
- Workspace root path
- Per-orchestrator private workspace override
- Shared workspace registry: logical name -> `{ path, description }`
- Per-orchestrator shared workspace allowlist
- Per-orchestrator config file path resolution and validation
- Heartbeat interval

## Settings Shape Requirements

Global config `~/.direclaw/config.yaml` must support:

- `workspaces_path`
- `shared_workspaces` object
  - keyed by logical shared workspace name
  - each value:
    - `path` absolute filesystem path
    - `description` required non-empty usage guidance
- `orchestrators` object keyed by orchestrator id
  - each orchestrator:
    - `private_workspace` (optional override; default `<workspaces_path>/<orchestrator_id>`)
    - `shared_access[]` (logical names from `shared_workspaces`)
- `channel_profiles` object keyed by channel profile id
  - each profile: `channel`, channel credentials/settings, `orchestrator_id`
  - `orchestrator_id` must reference `orchestrators.<orchestrator_id>`
  - for `slack` profiles include `slack_app_user_id` and `require_mention_in_channels`
- `monitoring` controls
- `channels` enablement controls
  - Slack channel runtime options:
    - `inbound_mode: socket|poll|hybrid` (default `socket`)
    - `socket_reconnect_backoff_ms`
    - `socket_idle_timeout_ms`
    - `history_backfill_enabled`
    - `history_backfill_interval_seconds`

Per-orchestrator config requirements:

- File location: `<resolved_orchestrator_private_workspace>/orchestrator.yaml`
- Required fields:
  - `id`
  - `selector_agent`
  - `workflows`
  - `default_workflow`
  - `selection_max_retries`
  - `agents` object keyed by agent id
- For each orchestrator-local agent:
  - `provider`, `model`, `can_orchestrate_workflows`
- Legacy agent fields are invalid and must fail fast:
  - `private_workspace`
  - `shared_access`
- For each workflow step:
  - `workspace_mode` supports only `orchestrator_workspace` and `run_workspace`
  - `agent_workspace` is invalid and must fail config validation
- `workflow_orchestration` safety defaults may be defined per orchestrator config
  - supported keys include:
    - `default_run_timeout_seconds`
    - `default_step_timeout_seconds`
    - `max_step_timeout_seconds`
    - `max_total_iterations`
- `workflows` must contain at least one valid workflow definition
- `default_workflow` must exist in `workflows`
- `selector_agent` must reference an agent in the same orchestrator config and must have `can_orchestrate_workflows: true`

Execution workspace behavior:

- Agent/provider executions run with CWD at `<resolved_orchestrator_private_workspace>`.
- Workflow run work areas resolve to `<resolved_orchestrator_private_workspace>/work/runs/<run_id>`.

Reference examples:

- `docs/build/spec/examples/settings/minimal.settings.yaml`
- `docs/build/spec/examples/settings/full.settings.yaml`
- `docs/build/spec/examples/orchestrators/minimal.orchestrator.yaml`
- `docs/build/spec/examples/orchestrators/engineering.orchestrator.yaml`
- `docs/build/spec/examples/orchestrators/product.orchestrator.yaml`

## Example Workspace Resolution

Given `workspaces_path` = `/Users/example/.direclaw/workspaces` and the example shared registry:

- `shared` -> `/Users/example/direclaw-shared`
- `docs` -> `/Users/example/company-docs`
- `data` -> `/Volumes/team-data`

Resolved orchestrator workspace examples:

- `engineering_orchestrator` private workspace: `/Users/example/.direclaw/workspaces/engineering_orchestrator`
- `engineering_orchestrator` shared access: `/Users/example/direclaw-shared`, `/Users/example/company-docs`
- `product_orchestrator` private workspace: `/Users/example/.direclaw/workspaces/product_orchestrator`
- `product_orchestrator` shared access: `/Users/example/company-docs`

## Orchestrator Commands

Required subcommands:

- `orchestrator list`
- `orchestrator add`
- `orchestrator show`
- `orchestrator remove`
- `orchestrator set-private-workspace <orchestrator_id> <abs_path>`
- `orchestrator grant-shared-access <orchestrator_id> <shared_key>`
- `orchestrator revoke-shared-access <orchestrator_id> <shared_key>`
- `orchestrator set-selector-agent <orchestrator_id> <agent_id>`
- `orchestrator set-default-workflow <orchestrator_id> <workflow_id>`
- `orchestrator set-selection-max-retries <orchestrator_id> <count>`

`orchestrator add` must:

- Create orchestrator private workspace.
- Bootstrap `<private_workspace>/orchestrator.yaml`.

Workflow management in orchestrator config must support:

- `workflow add <orchestrator_id> <workflow_id>`
- `workflow show <orchestrator_id> <workflow_id>`
- `workflow remove <orchestrator_id> <workflow_id>`
- `workflow list <orchestrator_id>`

`orchestrator show` must display:

- private workspace path
- shared-area access list

## Orchestrator Agent Commands

Required subcommands:

- `orchestrator-agent list <orchestrator_id>`
- `orchestrator-agent add <orchestrator_id> <agent_id>`
- `orchestrator-agent show <orchestrator_id> <agent_id>`
- `orchestrator-agent remove <orchestrator_id> <agent_id>`
- `orchestrator-agent reset <orchestrator_id> <agent_id>`

`orchestrator-agent add` must:

- Add agent definition to `<orchestrator_private_workspace>/orchestrator.yaml`.
- Set provider/model and capability flags in orchestrator config.

## Workflow Commands

Required subcommands:

- `workflow list <orchestrator_id>`
- `workflow show <orchestrator_id> <workflow_id>`
- `workflow run <orchestrator_id> <workflow_id> [--input key=value ...]`
- `workflow status <run_id>`
- `workflow progress <run_id>`
- `workflow cancel <run_id>`

Scope:

- Workflow definitions are scoped by orchestrator identity.
- Workflow commands must resolve target orchestrator scope explicitly (for example via `--orchestrator <orchestrator_id>`) or through deterministic caller context.

Authorization:

- Workflow starts must enforce `selector_agent` capability and `can_orchestrate_workflows` rules from orchestrator config.
- `workflow status` and `workflow progress` must be read-only operations and must never mutate run execution state.

## Channel Profile Commands

Required subcommands:

- `channel-profile list`
- `channel-profile add`
- `channel-profile show`
- `channel-profile remove`
- `channel-profile set-orchestrator <channel_profile_id> <orchestrator_id>`

`channel-profile show` must display:

- channel and credentials identity metadata
- mapped `orchestrator_id`
- effective mention policy (for slack profiles)

Slack channel command surface must include:

- `channels slack sync`
- `channels slack socket status`
- `channels slack socket reconnect`
- `channels slack backfill run`

## Provider and Model Commands

Required commands:

- `provider [anthropic|openai] [--model ...]`
- `model [sonnet|opus|haiku|gpt-5.3-codex|gpt-5.3-codex-spark]`

## Acceptance Criteria

- CLI supports full configuration lifecycle without manual YAML edits.
- Orchestrator workspace creation and per-orchestrator config bootstrap are automatic.
- Workflow command suite supports execution lifecycle from listing through cancel/status.
- Channel profiles can be configured and validated end-to-end, including orchestrator mapping and workflow behavior.

## Selector Function Exposure Contract

- All supported CLI commands in this spec and `docs/build/spec/10-daemon-operations.md` must be exposed as selector-callable functions for natural-language chat routing.
- Function registry must be machine-readable and passed to selector as `availableFunctions`.
- Each function entry must include:
  - stable `functionId`
  - argument schema (required/optional args and types)
  - short description for selector disambiguation
- `functionId` naming should be command-aligned (for example: `workflow.status`, `workflow.cancel`, `orchestrator.list`, `channel_profile.set_orchestrator`).
- Selector must not invoke functions outside `availableFunctions`.
