# Phase 02: Queue and Provider Execution

## Goal

Implement queue lifecycle, ordering guarantees, and provider CLI execution contract used by all agents.

## Tasks

### P02-T01 Implement queue claim/process/requeue lifecycle

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Incoming files are claimed in `mtime` FIFO order using atomic move to processing.
  - Success writes valid outgoing payload files with correct naming rules.
  - Failures trigger requeue attempts without payload loss.
- Automated Test Requirements:
  - Unit tests for queue filename rules and lifecycle transitions.
  - Integration test for `incoming -> processing -> outgoing` and forced failure requeue path.

### P02-T02 Implement per-key ordering and cross-key concurrency controls

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Strict sequential processing is preserved per `workflowRunId`.
  - Strict sequential processing is preserved per `(channel, channelProfileId, conversationId)` key.
  - Independent keys execute concurrently without head-of-line blocking.
- Automated Test Requirements:
  - Unit tests for key derivation and scheduler behavior.
  - Integration test with mixed keys proving sequence and concurrency guarantees.

### P02-T03 Implement provider runner for Anthropic and OpenAI CLIs

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Runner builds one CLI invocation per attempt with resolved provider/model/cwd.
  - Anthropic alias model mapping and OpenAI JSONL extraction behave per spec.
  - Timeout, non-zero exit, parse failure, and missing-binary errors are explicit and logged.
- Automated Test Requirements:
  - Unit tests for command construction, model mapping, and output parsing.
  - Integration tests with mocked CLI outputs for success/failure/timeout paths.

### P02-T04 Implement reset flag semantics and file-backed prompt assembly

- Status: `todo` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Global and per-agent reset flags are consumed exactly once after use.
  - Prompt/context artifacts are written to files before invocation.
  - Provider invocation logs include required metadata fields.
- Automated Test Requirements:
  - Unit tests for reset-flag precedence/consumption behavior.
  - Integration test verifying file-backed prompt flow and invocation logs.
