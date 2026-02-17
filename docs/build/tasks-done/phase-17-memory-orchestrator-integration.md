# Phase 17: Memory Orchestrator Integration

## Goal
Integrate memory read/write flows into selector, workflow-step, and diagnostics context assembly while preserving orchestrator control-plane guarantees.

## Tasks

### P17-T01 Integrate bulletin and recall into selector context builder

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Selector request context includes memory bulletin payload for each message.
  - Memory integration does not alter selector action/result schema contracts.
  - Failures in memory retrieval do not block selector execution.
- Automated Test Requirements:
  - Integration tests validating selector request artifacts include bulletin fields.
  - Regression tests ensuring selector still routes correctly under memory failure.

### P17-T02 Integrate memory context into workflow step prompt rendering

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Step prompt rendering receives bounded memory context bundle.
  - Context injection respects prompt size constraints and ordering rules.
  - Existing workflow output-path interpolation behavior remains unchanged.
- Automated Test Requirements:
  - Integration tests for prompt rendering with and without memory data.
  - Regression tests for unchanged workflow interpolation/output contracts.

### P17-T03 Add memory write-back from workflow outputs and transcripts

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Parsed workflow outputs can create/update typed memory records.
  - Channel transcript capture path can create memory observations/facts with provenance.
  - Write-back errors are isolated and logged without corrupting workflow run state.
- Automated Test Requirements:
  - Integration tests for post-step output memory creation.
  - Integration tests for transcript-driven memory persistence.

### P17-T04 Integrate diagnostics retrieval with memory evidence bundle

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Diagnostics flows can query bounded memory evidence and include provenance.
  - Diagnostics scope limits and existing safeguards remain enforced.
  - Memory evidence artifacts are persisted for replay/audit.
- Automated Test Requirements:
  - Integration tests for diagnostics intent using memory-backed evidence.
  - Regression tests for diagnostics scope limit enforcement.
