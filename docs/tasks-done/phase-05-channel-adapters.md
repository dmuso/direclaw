# Phase 05: Channel Adapters

## Goal

Implement Discord, Telegram, WhatsApp, and Slack adapters on shared queue contracts and channel-specific behavior.

## Tasks

### P05-T01 Build shared adapter framework (inbound mapping + outbound delivery)

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Adapters map inbound text/media to queue payload schema with `channelProfileId` where applicable.
  - Inbound media files are persisted to `~/.direclaw/files` with `[file: /abs/path]` tags.
  - Outbound processing sends files before text and cleans stale pending correlation entries.
- Automated Test Requirements:
  - Unit tests for payload mapping and pending-request cleanup policy.
  - Adapter integration tests for end-to-end inbound/outbound contract.

### P05-T02 Implement Discord, Telegram, and WhatsApp channel-specific behavior

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Discord handles DMs only and enforces 2000-char chunking with typing indicators.
  - Telegram handles private chats only, supported media types, and 4096-char chunking with typing indicators.
  - WhatsApp ignores group chats and manages auth session/QR/ready marker files.
- Automated Test Requirements:
  - Adapter tests for channel filters, media handling, and chunking limits.
  - Integration tests validating outbound responses for each adapter.

### P05-T03 Implement Slack multi-profile behavior and workflow-thread semantics

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Inbound events resolve `channelProfileId` deterministically and preserve credentials symmetry for outbound responses.
  - Channel processing respects DM/thread/allowlist/mention policy rules.
  - Workflow thread status/diagnostics requests route through orchestrator action handling contract.
- Automated Test Requirements:
  - Adapter tests for channel profile mapping and mention policy behavior.
  - Integration tests for thread reply semantics and status/diagnostics request handling.
