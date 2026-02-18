# Phase 18 Review: Unified Slack Target Handling

## Scope Reviewed
- Uncommitted changes in this workspace related to `docs/build/tasks/phase-18-unified-slack-target-handling.md`.
- Spec alignment checked against:
  - `docs/build/spec/03-agent-routing-execution.md`
  - `docs/build/spec/07-channel-adapters.md`
  - `docs/build/spec/15-scheduled-automation.md`
- Targeted test run in `nix-shell` for new/changed Slack-target and scheduler paths: all passed.

## Findings (Needs Action)

1. High: Egress allowlist enforcement now blocks valid non-DM thread replies in mentioned channels.
- Files:
  - `src/channels/slack/egress.rs:129`
  - `src/channels/slack/egress.rs:155`
- Problem:
  - `enforce_channel_policy` rejects any non-DM channel not in allowlist.
  - This check is applied to all outbound Slack deliveries, including normal thread replies.
  - Spec behavior allows channel processing when message is in thread OR channel is allowlisted OR app is mentioned (`docs/build/spec/07-channel-adapters.md`), and requires replies in thread for non-DM.
  - With current logic, if allowlist is configured and a message is accepted via mention/thread rule in a non-allowlisted channel, outbound reply can still be blocked.
- Why this matters:
  - Regresses normal Slack conversation behavior and can drop expected replies.
- Action:
  - Scope allowlist enforcement to explicit targeted channel posting flow (for Slack `targetRef` channel posts), not all ordinary thread-reply traffic.
  - Add integration coverage for: non-allowlisted channel + mention/thread-accepted inbound -> successful thread reply outbound.

2. High: `channelProfileId` and Slack `targetRef.channelProfileId` can diverge without rejection.
- Files:
  - `src/channels/slack/egress.rs:62`
  - `src/channels/slack/egress.rs:102`
- Problem:
  - Profile resolution prefers `outgoing.channel_profile_id` and returns early.
  - Delivery target resolution separately prefers `outgoing.target_ref` channel/thread.
  - If both exist but disagree, adapter can post to `targetRef.channelId` using different profile credentials.
- Why this matters:
  - Violates unified target contract expectations and can cause cross-profile/cross-orchestrator leakage behavior under malformed/forged queue payloads.
- Action:
  - Parse Slack target once and enforce consistency: if both profile fields are present, they must match or fail fast with deterministic error.
  - Add a regression test that crafts mismatched `channel_profile_id` vs `targetRef.channelProfileId` and asserts hard failure.

## Residual Notes
- No additional blocking issues found in scheduler-side Slack target schema validation, profile-orchestrator mapping checks, or queue propagation paths.
