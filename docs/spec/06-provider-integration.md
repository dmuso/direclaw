# Provider Integration (Anthropic/OpenAI)

## Scope

Defines local CLI invocation contracts, model mapping/pass-through behavior, output extraction, file-backed prompt assembly, and reset semantics.

## Execution Contract (All Agents)

Every agent run must execute exactly one configured provider CLI command per attempt.

This contract applies to:

- orchestrator workflow-selector agent executions
- workflow step agent executions
- orchestrator diagnostics investigation executions

Command construction inputs:

- `provider`: `orchestrator.yaml.agents.<agent_id>.provider`
- `model`: `orchestrator.yaml.agents.<agent_id>.model`
- `message`: fully rendered prompt/request text for that attempt
- `cwd`: resolved execution workspace for the attempt
- reset scope flags (global or per-agent)

Invocation rules:

- Spawn provider process with `cwd` set to resolved execution workspace.
- Capture `stdout` and `stderr`.
- Treat non-zero exit code as execution failure.
- Apply step/message timeout; timed-out processes must be terminated and marked failed.
- Do not send partial stdout directly to user channels.
- Prompt assembly must be file-backed:
  - runtime writes prompt and context artifacts to local files first
  - runtime then sends a compact instruction message that references those files
  - no non-file RPC channel may be used for provider context transfer

Response extraction:

- Extract final assistant response strictly from provider-specific output rules below.
- If extraction fails, treat attempt as failed and apply retry/requeue policy.

Selector-specific parsing contract:

- Orchestrator must prompt selector agent to emit one strict JSON result object.
- Result must parse into selection schema from `05-workflow-orchestration.md`.
- If selector output parse fails, mark selector attempt failed and apply selector retry policy.

Diagnostics-specific parsing contract:

- Orchestrator must prompt diagnostics agent to read persisted context bundle artifacts and produce natural-language findings.
- Diagnostics prompt must include expected response structure: likely cause, evidence summary, and next-step options.
- If diagnostics output is empty or unreadable, mark diagnostics attempt failed and apply diagnostics fallback behavior from `05-workflow-orchestration.md`.

## Provider: Anthropic

Command shape:

- `claude --dangerously-skip-permissions [--model mapped] [-c unless reset] -p <message>`

Model mapping:

- `sonnet` -> `claude-sonnet-4-5`
- `opus` -> `claude-opus-4-6`

Anthropic output handling:

- Use final textual completion emitted by `claude` process stdout as agent message.
- If stdout is empty or unreadable, mark attempt failed.

## Provider: OpenAI

Command shape:

- `codex exec [resume --last unless reset] [--model mapped] --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox --json <message>`

Model handling:

- Pass through supported names such as `gpt-5.2`, `gpt-5.3-codex`.

Output handling:

- Parse JSONL stream.
- Select final `item.completed` event where `item.type == "agent_message"`.
- Use extracted message payload as canonical agent response text.
- If no terminal `agent_message` is found, mark attempt failed.

## Reset Flags

Global reset flag:

- `~/.rustyclaw/reset_flag`

Per-agent reset flag:

- `<resolved_private_workspace>/reset_flag`

Behavior:

- If reset flag exists for run scope, next execution must start fresh conversation (no resume/continue).
- Reset flag is consumed and removed after it is used.

## Error Handling

- Missing provider binaries or invalid provider config must fail clearly and be logged.
- Provider output parse failures must fail message/workflow step and trigger existing retry/requeue policies.
- Unknown model aliases must be rejected by validation before invocation.
- Provider invocation logs must include:
  - resolved `agent_id`
  - provider type and model
  - command form (without sensitive tokens)
  - working directory
  - prompt/context file paths used for this invocation
  - exit code and timeout status

## Acceptance Criteria

- Anthropic alias models map correctly.
- OpenAI JSONL parsing consistently extracts terminal agent message event.
- Reset flags force fresh runs exactly once and are then deleted.
- Each agent capability path (selector, workflow task, workflow review, heartbeat, diagnostics investigation) is implemented via provider CLI invocation only.
- Diagnostics investigations use file-backed prompt/context assembly and provider CLI invocation only.
