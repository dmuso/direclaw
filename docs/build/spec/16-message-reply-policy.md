# Message Reply Policy

## Scope

Defines the end-state, channel-agnostic reply policy for inbound messages.

This policy is context-based and must not branch on channel name.

## Core Inbound Context

Every inbound message must carry normalized policy context:

- `is_direct` (`bool`)
- `is_mentioned` (`bool`)

`is_mentioned` wording (normative):

- `is_mentioned=true` means the inbound message explicitly addresses the assistant identity for the resolved channel profile.
- `is_mentioned=false` means no explicit assistant-addressing signal was detected for that message.

Adapter implementations may use adapter-specific detection mechanisms, but they must emit only the normalized boolean.

## Reply Decision Rules

The runtime must apply exactly these rules, in order:

1. If `is_direct=true`, reply.
2. Else if `is_mentioned=true`, reply.
3. Else, run opportunistic handling:
   - ingest the message as memory/knowledge, and
   - reply only when the selector chooses a reply-producing action.

No rule may depend on channel name.

## Routing and Selector Semantics

For rule 3 (`is_direct=false` and `is_mentioned=false`):

- The selector may choose `no_response`.
- The selector may choose a reply-producing action (for example `workflow_start`).
- The orchestrator must honor valid selector `no_response` for opportunistic messages.

For rules 1 and 2:

- `no_response` is not allowed.
- If selector returns `no_response`, orchestrator must override to the default reply-producing workflow path.

## Required Invariants

The system must satisfy all invariants:

- Changing channel name while keeping `is_direct`, `is_mentioned`, and selector result fixed must not change routing outcome.
- `is_direct=true` always yields a reply path.
- `is_mentioned=true` always yields a reply path.
- `is_direct=false` and `is_mentioned=false` allow `no_response`.

## Required Tests

At minimum, automated tests must cover:

- `reply_policy_direct_message_always_replies`
- `reply_policy_explicit_mention_always_replies`
- `reply_policy_non_direct_non_mentioned_allows_selector_no_response`
- `selector_no_response_produces_no_outgoing`
- `selector_workflow_start_produces_outgoing`
- `routing_outcome_is_independent_of_channel_name`

## Implementation Notes

- Core policy modules must use normalized inbound context (`is_direct`, `is_mentioned`) only.
- Adapter modules are responsible for deriving normalized context from transport-native events.
- Configuration must not require channel-specific identity abstractions to execute core policy.
