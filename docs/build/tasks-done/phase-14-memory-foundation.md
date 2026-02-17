# Phase 14: Memory Foundation

## Goal
Establish canonical memory subsystem scaffolding, strict configuration contracts, and per-orchestrator runtime path provisioning without changing existing queue/orchestrator behavior.

## Tasks

### P14-T01 Define memory runtime paths and bootstrap creation

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Runtime path helpers resolve `<orchestrator_runtime_root>/memory` subpaths deterministically.
  - Startup/provisioning creates required memory directories when missing.
  - Path creation failures return explicit, typed errors with path context.
- Automated Test Requirements:
  - Unit tests for path resolution and canonical path joining.
  - Integration tests that bootstrap a fresh orchestrator workspace and verify directory creation.

### P14-T02 Add typed memory configuration schema and validation

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Config supports `memory.enabled`, `memory.bulletin_mode`, retrieval settings, ingest settings, and scope settings.
  - Unknown keys or invalid types fail validation.
  - `memory.scope.cross_orchestrator=true` is rejected in v1.
- Automated Test Requirements:
  - Unit tests for valid and invalid config permutations.
  - Regression tests ensuring legacy/unknown shapes are rejected explicitly.

### P14-T03 Introduce core memory domain types

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Typed enums exist for all memory node and edge types in spec.
  - Bounds validation exists for `importance`, `confidence`, and edge `weight`.
  - Provenance/source model enforces required fields and scoped optional fields.
- Automated Test Requirements:
  - Unit tests for enum parsing/serialization stability.
  - Unit tests for bounds and required-field validation.

### P14-T04 Register memory worker lifecycle in runtime supervisor

- Status: `complete` (`todo|in_progress|complete`)
- Acceptance Criteria:
  - Memory worker starts/stops under existing runtime supervision model.
  - Worker health/status is observable via runtime state/logging paths.
  - Disabled memory config cleanly skips worker startup.
- Automated Test Requirements:
  - Integration tests for worker enable/disable behavior.
  - Integration tests for startup failure handling and error propagation.
