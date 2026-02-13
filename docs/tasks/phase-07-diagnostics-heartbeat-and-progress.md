# Phase 07: Diagnostics, Heartbeat, and Long-Run Progress

## Goal

Implement diagnostics investigation flow, heartbeat automation, and periodic progress updates for active runs.

## Tasks

### P07-T01 Implement diagnostics scope resolution and bounded context gathering

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `diagnostics_investigate` resolves target run scope via defined precedence.
  - Context retrieval obeys file/log count and size/time bounds.
  - Ambiguous scope returns a single clarifying question instead of guessing.
- Automated Test Requirements:
  - Unit tests for scope resolver precedence and ambiguity handling.
  - Integration test for bounded diagnostics bundle generation and unresolved-scope fallback.

### P07-T02 Implement diagnostics provider reasoning and audit artifact persistence

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Diagnostics prompts run through provider CLI using file-backed context bundle.
  - Results persist context, tool trace, invocation metadata, and user-facing response artifacts.
  - Diagnostics loops enforce max reasoning turns and retrieval rounds.
- Automated Test Requirements:
  - Unit tests for diagnostics result schema and loop limits.
  - Integration tests for successful and insufficient-confidence diagnostics outcomes.

### P07-T03 Implement heartbeat automation worker

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Heartbeat scheduler enqueues per-agent messages at configured interval.
  - Missing `heartbeat.md` falls back to default prompt without failure.
  - Matching outbound heartbeat responses are logged for observability.
- Automated Test Requirements:
  - Unit tests for schedule timing and fallback prompt selection.
  - Integration test for heartbeat enqueue/response-log cycle.

### P07-T04 Implement periodic run-progress heartbeat and Slack 15-minute thread posts

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Active runs refresh `progress.json` at least once per minute.
  - Slack-originated active runs emit progress posts every 15 minutes and final terminal post.
  - Status snapshots include required fields (`runId`, state, step, elapsed, summary).
- Automated Test Requirements:
  - Unit tests for heartbeat cadence and post eligibility logic.
  - Integration tests for in-thread Slack progress posting cadence and final post emission.
