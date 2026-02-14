# Phase 03: Slack Runtime Worker

## Goal

Operate Slack as a daemon-managed worker with stable inbound/outbound behavior, profile health reporting, and conversation correctness.

## Plan Context

Primary reference:
- `docs/build/release-readiness-plan.md`

## Tasks

### P03-T01 Integrate Slack worker into supervisor lifecycle

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Start Slack worker automatically when `channels.slack.enabled=true`.
  - Reuse existing sync logic while converting to periodic worker loop.
  - Ensure worker startup validates required env credentials.
- Acceptance Criteria:
  - Slack worker starts/stops with daemon lifecycle commands.
  - Missing credentials fail with explicit profile-scoped error details.
- Automated Test Requirements:
  - Integration tests for worker start with valid/missing env configurations.

### P03-T02 Preserve conversation/thread mapping for inbound messages

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Verify enqueue format includes `channel_profile_id` and stable conversation identifiers.
  - Ensure mention policy and allowlist behavior remain deterministic.
  - Keep idempotent behavior for already-seen messages via cursor state.
- Acceptance Criteria:
  - Inbound channel/thread behavior matches configured mention and allowlist policy.
  - Duplicate ingestion is avoided on repeated sync cycles.
- Automated Test Requirements:
  - Integration tests covering DM, mention-required channel, allowlist channel, and duplicate polling.

### P03-T03 Stabilize outbound delivery and failure handling

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Ensure outbound queue files for Slack are delivered and removed on success.
  - Keep chunking/thread posting behavior and explicit failure logs.
  - Add retry-friendly behavior for transient API failures.
- Acceptance Criteria:
  - Successful outbound sends clear queue entries.
  - Failed sends preserve actionable error context.
- Automated Test Requirements:
  - Integration tests for chunked outbound delivery and API failure paths.

### P03-T04 Expose profile-level readiness in `status`

- Status: `todo` (`todo|in_progress|complete`)
- Implementation Notes:
  - Add profile readiness/health details to status output.
  - Include explicit reasons (disabled channel, auth missing, API failure, running).
- Acceptance Criteria:
  - `status` clearly identifies each configured Slack profile health state.
- Automated Test Requirements:
  - CLI integration tests asserting profile-specific health lines.

