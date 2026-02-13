# Phase 01: Runtime and Configuration Foundation

## Goal

Establish state root layout, worker process model, settings/orchestrator config loading, and startup validation.

## Tasks

### P01-T01 Implement runtime bootstrap and required filesystem layout

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Startup creates all required `~/.direclaw` queue/orchestrator/workflow/log paths if missing.
  - Polling defaults are set to 1s for queue and outbound channel processing.
  - Worker registry supports independent lifecycle for queue, orchestrator, adapters, and optional heartbeat.
- Automated Test Requirements:
  - Unit tests for path resolver and directory bootstrap logic.
  - Integration test that bootstraps from empty state root and verifies full directory tree exists.

### P01-T02 Implement settings and orchestrator config schema validation

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - `settings.yaml` required domains and references validate with explicit errors.
  - `<orchestrator_private_workspace>/orchestrator.yaml` required fields validate.
  - `selector_agent`, `default_workflow`, and non-empty workflow rules are enforced.
- Automated Test Requirements:
  - Unit tests for config parsing and cross-reference validation.
  - Integration test loading minimal and full example configs successfully.

### P01-T03 Implement workspace path resolution and shared-workspace grants

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Deterministic private workspace resolution uses override or default rule.
  - Shared workspace registry paths are absolute and canonicalized.
  - Invalid or missing shared paths fail fast during validation.
- Automated Test Requirements:
  - Unit tests for workspace resolution precedence and canonicalization.
  - Integration test asserting deny-by-default shared access behavior.
