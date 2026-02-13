# Workflow Orchestration

## Scope

Defines orchestrator-owned workflow execution across agents, definition schema, workspace model, control-plane contract, and deterministic routing.

## Core Model

- Workflows are orchestrator-managed.
- All channel-originated messages are orchestrator-dispatched.
- Agents do not directly message each other as a standalone feature.
- Orchestrator controls:
  - step ordering
  - step input/output mapping
  - approval/rejection gates
  - retry and timeout handling

Persisted run state:

- `~/.direclaw/workflows/runs/<run_id>.json`
- Valid run states: `queued`, `running`, `waiting`, `succeeded`, `failed`, `canceled`

Run progress snapshot:

- `~/.direclaw/workflows/runs/<run_id>/progress.json`
- Must be updated at workflow start, step-attempt start/end, state transitions, and periodic heartbeat ticks while `state=running|waiting`
- Must include:
  - `runId`
  - `workflowId`
  - `state`
  - `currentStepId` (when active)
  - `currentAttempt` (when active)
  - `startedAt`
  - `updatedAt`
  - `lastProgressAt`
  - `summary` (short plain-text status line)
  - `pendingHumanInput` (`true|false`)
  - `nextExpectedAction` (short plain text)

Supported step types:

- `agent_task`
- `agent_review`

Workflow step execution mechanism:

- Every step attempt is executed by invoking the step agent's configured provider CLI (`claude` or `codex`) using `docs/spec/06-provider-integration.md`.
- Orchestrator never bypasses provider CLI execution for any step type.

## Channel Entry Routing

For channel-originated messages without `workflowRunId`, orchestrator must run workflow selection before starting a run.

Required config split:

- Global `settings.yaml`:
  - `channel_profiles.<channel_profile_id>.orchestrator_id`
  - `orchestrators.<orchestrator_id>.private_workspace`
- Per-orchestrator `<orchestrator_private_workspace>/orchestrator.yaml`:
  - `selector_agent`
  - `workflows`
  - `default_workflow`
  - `selection_max_retries`

Selection rules:

- Selector receives original user message plus workflows discovered from `orchestrator.yaml.workflows`.
- Selector may choose one supported action:
  - `workflow_start` (and one workflow id from discovered workflow ids)
  - `workflow_status` (status report request for current conversation context)
  - `diagnostics_investigate` (diagnose failure or runtime behavior for current conversation context)
  - `command_invoke` (invoke one supported DireClaw function from function registry)
- `workflows` must be non-empty.
- `default_workflow` must exist in `workflows`.
- If selector output is invalid or selection fails after retries, orchestrator starts `default_workflow`.
- `default_workflow` must be one of discovered workflow ids.

Function registry requirements for selector:

- Selector context must include `availableFunctions` derived from supported CLI command surfaces.
- Selector may choose only function ids present in `availableFunctions`.
- Function ids must be stable machine identifiers (for example `workflow.status`, `workflow.cancel`, `orchestrator.list`).

Default subject-matter deployment pattern:

- One channel profile per domain (for example: engineering, product, strategy, sales)
- Each orchestrator defines inline workflows plus a domain-specific default workflow
- Domain default workflow can be a minimal orchestrator -> expert worker pattern
- Per-orchestrator selectors/workflows/agents are declared in `<orchestrator_private_workspace>/orchestrator.yaml`

Orchestrator config examples:

- `docs/spec/examples/orchestrators/minimal.orchestrator.yaml`
- `docs/spec/examples/orchestrators/engineering.orchestrator.yaml`
- `docs/spec/examples/orchestrators/product.orchestrator.yaml`

## Workflow Definition Format

Storage:

- Workflow definitions are scoped per orchestrator.
- For resolved orchestrator `<orchestrator_id>`, definitions are loaded from:
  - `orchestrator.yaml.workflows`
- Workflow ids are unique within a single orchestrator scope.
 
Required per-workflow fields:

- `id`, `version`, `inputs`, `steps`

Step required fields:

- `id`, `type`, `agent`, `prompt`

Optional fields:

- `needs`, `outputs`, `next`
- `limits.max_retries`
- `workspace_mode` (`orchestrator_workspace` default, `run_workspace`, `agent_workspace`)
- For review steps: `on_approve`, `on_reject`
- For output-producing steps: `output_files` is required whenever `outputs` is present
  - `output_files` must map every key in `outputs`
  - each mapped value must be a relative file-path template

Reference examples:

- `docs/spec/examples/orchestrators/minimal.orchestrator.yaml`
- `docs/spec/examples/orchestrators/engineering.orchestrator.yaml`
- `docs/spec/examples/orchestrators/product.orchestrator.yaml`

## Template Interpolation

Prompts and workflow expressions must support:

- `inputs.*`
- `steps.<step_id>.outputs.*`
- mutable run `state.*`
- runtime `workflow.*` context

Runtime context includes:

- `workflow.run_id`
- `workflow.step_id`
- `workflow.attempt`
- `workflow.run_workspace`
- `workflow.output_schema_json`
- `workflow.output_paths_json`
- `workflow.output_paths.<key>`
- `workflow.channel`
- `workflow.channel_profile_id`
- `workflow.conversation_id`
- `workflow.sender_id`
- `workflow.selector_id`

Recommended selected-workflow `inputs` for channel dispatch:

- `user_message`
- `sender`
- `sender_id`
- `channel`
- `conversation_id`
- `channel_profile_id`
- `files`

`output_files` templates may interpolate only:

- `workflow.run_id`
- `workflow.step_id`
- `workflow.attempt`

## Workflow Selection Control Plane

Selector request persistence:

- `~/.direclaw/orchestrator/messages/<message_id>.json` (canonical normalized message snapshot)
- `~/.direclaw/orchestrator/select/incoming/<selector_id>.json`
- `~/.direclaw/orchestrator/select/processing/<selector_id>.json`

Selector result persistence:

- `~/.direclaw/orchestrator/select/results/<selector_id>.json`
- `~/.direclaw/orchestrator/select/logs/<selector_id>.log`

Selector request JSON must include:

- `selectorId`
- `channelProfileId`
- `messageId`
- `conversationId` (when available)
- `userMessage`
- `availableWorkflows` (non-empty array)
- `defaultWorkflow`

Selector result JSON must include:

- `selectorId`
- `status` (`selected`|`failed`)
- `action` (`workflow_start`|`workflow_status`|`diagnostics_investigate`|`command_invoke`) (required when `status=selected`)
- `selectedWorkflow` (required when `status=selected` and `action=workflow_start`)
- `diagnosticsScope` (object; required when `status=selected` and `action=diagnostics_investigate`)
- `functionId` (required when `status=selected` and `action=command_invoke`)
- `functionArgs` (object; required when `status=selected` and `action=command_invoke`, empty object allowed)

Workflow run metadata for selector-started runs must include:

- `sourceMessageId`
- `selectorId`
- `selectedWorkflow`
- `statusConversationId` (channel conversation/thread id used for progress updates)

Selector validation rules:

- `action` must be one of `workflow_start`, `workflow_status`, `diagnostics_investigate`, `command_invoke`.
- `selectedWorkflow` must be in `availableWorkflows` when action is `workflow_start`.
- `diagnosticsScope` must be valid JSON object when action is `diagnostics_investigate`.
- `diagnosticsScope` may include optional `runId`, `stepId`, and `timeWindowMinutes`.
- `functionId` must be in `availableFunctions` when action is `command_invoke`.
- `functionArgs` must be valid JSON object when action is `command_invoke`.
- Unknown workflow ids are invalid.
- Non-JSON or malformed JSON results are invalid.
- Invalid result increments selector retry counter.

## Selector Prompt Templates

Selector prompts must use a deterministic template and require strict JSON output.

Template references:

- `docs/spec/examples/prompts/workflow_selector_minimal.prompt.md`
- `docs/spec/examples/prompts/workflow_selector_rich.prompt.md`

Required prompt inputs:

- `channel_profile_id`
- `message_id`
- `conversation_id` (when available)
- `user_message`
- `available_workflows` with short descriptions
- `default_workflow`

Required output instruction:

- Output only one JSON object with keys:
  - `selectorId`
  - `status` (`selected`|`failed`)
  - `action` (`workflow_start`|`workflow_status`|`diagnostics_investigate`|`command_invoke`) (required when `status=selected`)
  - `selectedWorkflow` (required when `status=selected` and `action=workflow_start`)
  - `diagnosticsScope` (required when `status=selected` and `action=diagnostics_investigate`)
  - `functionId` (required when `status=selected` and `action=command_invoke`)
  - `functionArgs` (required when `status=selected` and `action=command_invoke`)
  - `reason` (short plain text, max 200 chars)

Prompt constraints:

- Selector must pick exactly one supported action.
- If action is `workflow_start`, selector must pick exactly one workflow from `available_workflows` when possible.
- If action is `diagnostics_investigate`, selector must provide a bounded `diagnosticsScope`.
- If action is `command_invoke`, selector must pick exactly one function id from `available_functions`.
- Selector must not emit markdown fences.
- Selector must not emit additional prose outside JSON.

## Diagnostics Investigation Flow

`diagnostics_investigate` is the orchestrator-owned path for natural-language requests such as "why did this fail?" and "investigate what failed."

Execution contract:

- Must not advance workflow step execution state.
- Must run in two phases:
  1. deterministic context gathering
  2. diagnostics agent reasoning over gathered context

Scope rules:

- Diagnostics scope is resolved in this order:
  1. explicit `diagnosticsScope.runId` when present
  2. inbound `workflowRunId` when present
  3. active run association for `(channelProfileId, conversationId)`
- If scope cannot be resolved, runtime must ask a clarifying question instead of guessing target run.
- All file reads must be restricted to:
  - `~/.direclaw/workflows/runs/<run_id>`
  - `~/.direclaw/orchestrator/select`
  - `~/.direclaw/logs`
  - resolved orchestrator private workspace
- Path traversal outside allowed roots is invalid and must be blocked and logged.

Deterministic context gathering:

- Gather and persist a diagnostics context bundle:
  - running workflows and run states relevant to conversation
  - available workflows from orchestrator config
  - bounded file tree snapshots for run workspace and outputs
  - relevant result artifacts (`progress.json`, step `result.json`, selector result)
  - bounded log excerpts filtered by run/conversation identifiers and recent window
- Persist gathered bundle at:
  - `~/.direclaw/orchestrator/diagnostics/context/<diagnostics_id>.json`

Retrieval policy:

- Retrieval must be ranked and bounded:
  - max files per diagnostics request: 20
  - max log excerpt blocks: 10
  - max bytes per file excerpt: 64 KB
  - default log time window: last 120 minutes unless overridden by bounded `diagnosticsScope.timeWindowMinutes`
- Prefer artifacts directly tied to active/failed step over global logs.
- Retrieval must include provenance for each excerpt (`path`, `offset` or `time_range`, `reason_selected`).

Tool contracts (orchestrator-internal):

- `diagnostics.resolve_scope(input_message)` -> `{ runId|unresolved_reason }`
- `diagnostics.list_running_runs(scope)` -> `{ runs[] }`
- `diagnostics.list_available_workflows(orchestrator_id)` -> `{ workflows[] }`
- `diagnostics.snapshot_filesystem(scope)` -> `{ trees[] }`
- `diagnostics.collect_artifacts(scope)` -> `{ artifacts[] }`
- `diagnostics.collect_logs(scope, window)` -> `{ excerpts[] }`
- `diagnostics.ask_clarifying_question(unresolved_reason)` -> `{ question }`
- `diagnostics.compose_response(findings)` -> `{ message, suggested_next_steps[] }`

All tool outputs must be serializable JSON and persisted under:

- `~/.direclaw/orchestrator/diagnostics/results/<diagnostics_id>.json`

Looping and fallback controls:

- Maximum diagnostics reasoning turns per inbound message: 3
- Maximum additional retrieval rounds requested by diagnostics agent: 2
- If confidence is insufficient or scope is ambiguous after limits, runtime must return:
  - what was checked
  - what is missing
  - one clarifying question

Auditability:

- Every diagnostics request must persist:
  - normalized inbound message snapshot
  - resolved scope decision and alternatives considered
  - context bundle
  - diagnostics prompt file path and provider invocation metadata
  - tool call trace and selected excerpts
  - final user-facing response
- Diagnostics logs must be written to:
  - `~/.direclaw/orchestrator/diagnostics/logs/<diagnostics_id>.log`

## Execution Workspace Model

Run workspace root:

- `~/.direclaw/workflows/runs/<run_id>/workspace/`

Per-step attempt output root:

- `~/.direclaw/workflows/runs/<run_id>/steps/<step_id>/attempts/<attempt>/outputs/`

Rules:

- `workspace_mode: orchestrator_workspace` executes in the resolved orchestrator private workspace (default when omitted).
- `workspace_mode: run_workspace` executes inside run workspace.
- `workspace_mode: agent_workspace` executes in agent private/shared context.
- Output paths are precomputed deterministically by orchestrator from `output_files`.
- Each attempt has distinct output root and canonical paths.
- All output paths must resolve under step output root; traversal is invalid.

## Worker-Orchestrator Control Plane

Each worker final message must include a strict machine-readable block:

- Begin marker: `[workflow_result]`
- End marker: `[/workflow_result]`
- Body: strict JSON

This control block is the only source of truth for step routing and step outputs.

Natural language outside this block is ignored for routing.

Required JSON envelope:

```json
{
  "status": "complete|blocked|failed",
  "summary": "short step summary",
  "output_files_written": true,
  "changed_files": ["relative/path/if_any"],
  "test_report": {
    "status": "passed|failed|not_run",
    "command": "optional",
    "details": "optional"
  }
}
```

Output obligations:

- Worker must write every declared output to exact orchestrator-provided path.
- Worker must not choose alternative output locations.
- Output formats are step-contract-driven and explicitly described in prompts.

Review-step output file rules:

- `decision`: UTF-8 text, one token exactly `approve` or `reject` (case-insensitive parse)
- `feedback`: UTF-8 Markdown text
- `approved_plan` (when required): UTF-8 Markdown text

Validation checks per required output:

- Exists
- Inside step output root
- Readable
- Non-empty unless step config allows empty

Invalid JSON envelope, missing outputs, invalid paths, or unreadable files:

- Step fails
- Retry policy applies

Control payload persistence:

- `~/.direclaw/workflows/runs/<run_id>/steps/<step_id>/result.json`

## Deterministic Routing Rules

Routing ownership is always orchestrator-owned.

Channel entry routing:

- If `workflowRunId` exists, continue existing run.
- Else run selector using channel profile orchestrator config.
- If selector succeeds with `action=workflow_start`, start selected workflow.
- If selector succeeds with `action=workflow_status`, resolve status using `(channelProfileId, conversationId)` association and return latest progress snapshot without advancing workflow steps.
- If selector succeeds with `action=diagnostics_investigate`, execute diagnostics investigation flow and return natural-language findings without advancing workflow steps.
- If selector succeeds with `action=command_invoke`, execute the resolved function with validated arguments and return command result payload to channel.
- If selector fails after retries, start default workflow.
- Execute first step only when resolved action is `workflow_start`.

User status-check routing:

- For inbound messages on an active workflow conversation/thread:
  - Intent interpretation for natural-language status requests must be performed by orchestrator selector-agent inference (provider CLI), not keyword-only matching.
  - Exact commands (`status`, `progress`, `/status`, `/progress`, case-insensitive) may use a deterministic fast-path and must produce the same routing result as selector classification.
  - For selector action `workflow_status`, run resolution precedence is:
    1. If `workflowRunId` is present, use that run id (system-facing precision).
    2. Else resolve from active run association for `(channelProfileId, conversationId)` (user-facing lookup).
    3. Else return a deterministic "no active workflow run found for this conversation" response.
  - Runtime must respond using latest `progress.json` snapshot without triggering a new workflow step execution.
  - Response must include current state, active/pending step id, latest summary, and elapsed runtime.

User diagnostics routing:

- For inbound messages interpreted as failure/root-cause investigation:
  - Intent interpretation must be performed by orchestrator selector-agent inference.
  - Selector should use `diagnostics_investigate`.
  - Runtime must execute diagnostics investigation flow.
  - Response must be natural language and include:
    - likely failure cause (or uncertainty statement)
    - evidence summary with artifact/log references
    - immediate suggested next steps
  - If scope is ambiguous, ask a single clarifying question.

`agent_task` routing:

- `status=complete` -> `next` or workflow end
- `status=blocked|failed` -> `on_blocked`/`on_failed` if configured, else run fails

`agent_review` routing:

- Read decision from mapped `decision` output file
- Parse UTF-8, trim whitespace, lowercase
- Must equal `approve` or `reject`
- `approve` -> `on_approve`
- `reject` -> `on_reject`

Missing/invalid decision:

- Retry up to `limits.max_retries`
- Then fail run

## Loop and Safety Controls

Required controls:

- per-step retry limits
- max total iterations per run
- run-level timeout
- step-level timeout (global default with optional per-step override)

Timeout precedence and clamp rules:

- Effective step timeout resolves in this order:
  1. `steps[].limits.timeout_seconds` (when present)
  2. `workflow_orchestration.default_step_timeout_seconds`
- Effective step timeout must not exceed `workflow_orchestration.max_step_timeout_seconds` when that max is configured.
- If a configured per-step timeout exceeds the max, runtime must clamp to max and log the clamp event in run diagnostics.

Unauthorized workflow start attempts must be rejected and logged.

Orchestrator directives:

- Allowed from orchestrator-capable agents.
- Dispatch directives must be stripped from final user-visible channel text.

## Long-Running Progress Monitoring

Long-running run definition:

- Any run that remains in `running` or `waiting` state for more than 15 minutes from `startedAt`.

Required monitoring behavior:

- Orchestrator must evaluate active runs at least once per minute.
- For each active run, orchestrator must refresh `progress.json` and maintain `lastProgressAt`.
- If no step output changed since last check, orchestrator still updates heartbeat fields (`updatedAt`, `lastProgressAt`) to prove liveness.

Slack thread progress posting:

- For runs originating from Slack, orchestrator must post progress updates to the associated Slack thread/conversation identified by run metadata.
- Post cadence is mandatory: every 15 minutes (900 seconds) while run state is `running` or `waiting`.
- A final completion post is required on transition to `succeeded`, `failed`, or `canceled`.
- Progress post content must include:
  - `runId`
  - workflow state
  - active or last completed step id
  - elapsed time
  - short summary from `progress.json.summary`
- Progress posts are additive thread messages; they must not overwrite prior thread history.

## Acceptance Criteria

- Multi-step workflows persist state transitions and per-step results.
- Approval/rejection loops behave deterministically.
- Retries and timeout limits terminate runaway workflows.
- Output path traversal attempts are blocked and logged.
- Active runs expose machine-readable progress snapshots.
- Workflow status-check commands return current run progress without advancing workflow steps.
- Slack-originated runs receive progress updates in-thread every 15 minutes until terminal state.
- Diagnostics investigations run with bounded retrieval, persisted audit artifacts, and no workflow-step advancement.
