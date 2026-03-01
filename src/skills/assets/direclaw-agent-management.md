---
name: direclaw-agent-management
description: "Use when adding, updating, or removing DireClaw agents in orchestrator configuration."
---

# DireClaw Agent Management

## Purpose
Manage agent definitions only.

## DireClaw Context
- In DireClaw, an Agent is a configured runtime wrapper (for `codex` or `claude` execution).
- Workflows assign one agent per step.
- Selector/default workflow behavior depends on valid agent references.
- Agent execution command construction uses:
  - `provider` -> which CLI binary is invoked
  - `model` -> `--model` value (mapped or pass-through)
  - `can_orchestrate_workflows` -> selector/orchestrator capability flag

## What To Do
1. If the request is about agents, edit only orchestrator config keys related to agents.
2. When editing an agent, set explicit `provider` and `model` values and change only the requested agent fields.
3. Validate that workflows/selectors still reference valid agent ids.
4. Fix any invalid references immediately.

## Allowed Values
- `provider`:
  - `anthropic`
  - `openai`
- `model`:
  - For `anthropic`: `sonnet`, `opus`, `haiku`
  - For `openai`: `gpt-5.3-codex`, `gpt-5.3-codex-spark`
- `can_orchestrate_workflows`:
  - `true` or `false`
  - Controls whether an agent is allowed to perform workflow orchestration actions (for example, selector/default-workflow routing decisions).
  - Does not change provider/model execution behavior for normal workflow steps; it is an orchestration capability gate.
  - Selector agent must have `can_orchestrate_workflows: true`.
  - Use `false` for agents that should only execute assigned steps and never orchestrate workflows.

## Where To Change
- Agent definitions live in:
  - `<current_working_directory>/orchestrator.yaml`
- Agent section shape:
```yaml
agents:
  <agent_id>:
    provider: anthropic|openai
    model: <model_id>
    can_orchestrate_workflows: true|false
```
- Use the orchestrator private workspace path already in scope for the task.

## Examples on How To Choose a Model
- Router/selector agents: `gpt-5.3-codex-spark` or `haiku`
- Simple/fast tasks: `gpt-5.3-codex-spark` or `haiku`
- Code planning: `gpt-5.3-codex` or `opus`
- Research: `gpt-5.3-codex` or `opus`
- Complex decision making: `gpt-5.3-codex` or `opus`
- Medium difficulty tasks: `gpt-5.3-codex` or `sonnet`
- Coding/build agents: prefer OpenAI `gpt-5.3-codex`
- Concise, logical reasoning tasks: prefer OpenAI `gpt-5.3-codex`
- Creative tasks (eg: writing, branding, marketing): prefer Anthropic `opus` or `sonnet`
- UI design: prefer Anthropic `opus`
- Code/plan review: use alternate provider, ie: if plan/build is OpenAI, then use Anthropic to review
- Follow existing orchestrator patterns unless the user requests a capability change.

## How To Verify
1. Static config check:
   - `direclaw orchestrator-agent show <orchestrator_id> <agent_id>`
2. Invariant check:
   - Ensure selector agent exists and has `can_orchestrate_workflows: true`.
   - Ensure no workflow step references a missing agent.

## What Not To Do
- Do not invent input contracts that the runtime does not enforce.
- Do not make unrelated mount or workspace structure changes.

## Deliverable
Return:
- Agent definitions changed/created/deleted.
- Any broken references fixed.
