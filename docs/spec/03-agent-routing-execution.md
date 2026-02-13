# Agent Routing and Execution

## Scope

Defines orchestrator-owned routing and agent-step execution constraints.

## Orchestrator-First Routing

For channel-originated queued messages, routing ownership is orchestrator-owned in this order:

1. If `workflowRunId` is present, route to that run.
2. Else resolve `channelProfileId` from message context.
3. Resolve `orchestrator_id` from `channel_profiles.<channelProfileId>.orchestrator_id`.
4. Load orchestrator config from `<resolved_orchestrator_private_workspace>/orchestrator.yaml`.
5. Build selector request using:
   - original message payload
   - `orchestrator.yaml.selector_agent`
   - `orchestrator.yaml.workflows`
   - `orchestrator.yaml.default_workflow`
6. Invoke selector agent via provider CLI and parse selector result.
7. If selector action is `workflow_start` and chosen workflow is valid in resolved orchestrator workflow set, start that workflow.
8. If selector action is `workflow_status`, resolve current run status for the channel conversation and return progress snapshot without advancing workflow steps.
9. If selector action is `diagnostics_investigate`, run diagnostics context gathering plus diagnostics agent inference and return natural-language findings without advancing workflow steps.
10. If selector action is `command_invoke`, execute the resolved function with validated arguments.
11. If selector output is invalid or selection fails after configured retries, start `default_workflow`.

Provider execution occurs at both orchestrator selector stage and workflow step stage.

## Execution Semantics

- Each workflow step resolves exactly one execution agent.
- Channel-originated execution must use queue guarantees from `02-queue-processing.md` and orchestrator routing from `05-workflow-orchestration.md`.
- Workflow-bound messages (`workflowRunId`, `workflowStepId`) remain orchestrator-owned.

## CLI-Only Agent Capability Model

All agentic behavior is implemented by invoking provider CLIs (`claude` or `codex`).

Rules:

- RustyClaw must not implement alternate internal reasoning/execution engines for agents.
- For every resolved agent execution, runtime selects provider from `orchestrator.yaml.agents.<agent_id>.provider`.
- Runtime builds one provider CLI command per execution attempt and captures its output.
- Agent response content is derived only from provider CLI output parsing rules.
- Any capability such as planning, coding, reviewing, or workflow participation is expressed through prompt content passed to the provider CLI.

Execution context passed to every invocation:

- resolved agent id
- resolved working directory (private workspace or workflow-selected workspace)
- input message/prompt text
- attached file tags and workflow context when applicable
- model from `orchestrator.yaml.agents.<agent_id>.model` (mapped or pass-through by provider rules)

## Validation and Failure Rules

- If no agents are configured, processing must fail loudly with actionable configuration error logging.
- If provider configuration is missing/invalid for resolved agent, execution must fail with explicit agent-scoped error logging.
- Missing/invalid `channelProfileId` or missing orchestrator config for channel traffic must fail with explicit channel-profile-scoped errors.
- `orchestrator_id` must reference `settings.orchestrators.<orchestrator_id>`.
- `<orchestrator_private_workspace>/orchestrator.yaml` must exist and parse.
- `selector_agent` in `orchestrator.yaml` must reference a configured orchestrator-local agent id.
- `selector_agent` in `orchestrator.yaml` must have `can_orchestrate_workflows: true`.
- `workflows` must be non-empty in `orchestrator.yaml`.
- `default_workflow` must exist in `orchestrator.yaml.workflows`.
- Selector output must be strict JSON and must resolve to one supported selector action.
- If selector action is `workflow_start`, output must resolve to exactly one workflow id from resolved orchestrator workflow set.
- If selector action is `diagnostics_investigate`, output must resolve target run scope by `workflowRunId` when present or by active `(channelProfileId, conversationId)` association.
- If selector action is `command_invoke`, `functionId` must resolve to exactly one supported function from selector-provided `availableFunctions`.
- If selector output references an unknown workflow id, execution must fail validation and follow retry/default rules.

## Acceptance Criteria

- Channel-originated messages resolve through orchestrator workflow routing deterministically.
- Runtime logs include routing source (`workflowRunId`, `channelProfileId`, selector_choice|default_workflow).
- Runtime logs include provider binary invoked and resolved execution workspace for each attempt.
- Runtime logs include diagnostics request id and resolved diagnostics scope when selector action is `diagnostics_investigate`.
