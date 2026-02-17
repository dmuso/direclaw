# Phase 14 Memory Foundation Review

Reviewed scope: uncommitted changes in workspace against `docs/build/spec/14-memory.md` and task acceptance criteria in `docs/build/tasks/phase-14-memory-foundation.md`.

## Findings Requiring Action

1. High: Provenance `source_path` is not canonicalized, only checked for absoluteness.
- Spec gap: `source.sourcePath` must be an absolute canonical path when file-backed (`docs/build/spec/14-memory.md`, Provenance and Auditability + Unit Tests section).
- Evidence: `MemorySource::validate` only enforces `is_absolute()` and never canonicalizes or validates canonical equivalence in `src/memory/domain.rs:109`.
- Required action: Add canonicalization validation for file-backed source paths (at minimum for `ingest_file`, and any other file-backed source types once defined), and add unit tests covering canonical success/failure cases.

2. High: Memory domain serialization field names do not match spec contract naming.
- Spec gap: Core model fields are defined as `memoryId`, `orchestratorId`, `edgeId`, `fromMemoryId`, `sourceType`, etc. in `docs/build/spec/14-memory.md`.
- Evidence: structs currently serialize/deserialze snake_case fields (`memory_id`, `orchestrator_id`, `edge_id`, `source_type`, etc.) without `serde` renaming in `src/memory/domain.rs:95`, `src/memory/domain.rs:155`, and `src/memory/domain.rs:193`.
- Required action: Apply explicit serde naming (`rename_all = "camelCase"` or per-field renames) to align with spec-defined contract and add round-trip tests asserting exact JSON field names.

3. Medium: Missing integration assertion for runtime bootstrap creating required memory directories and logging path outputs.
- Task gap: P14-T01 requires integration coverage for bootstrapping a fresh orchestrator workspace and verifying directory creation; P14-T04 expects worker observability via runtime state/logging paths.
- Evidence: `tests/runtime_memory_foundation.rs:37` verifies worker registration/health but does not assert that `memory/ingest`, `memory/ingest/processed`, `memory/ingest/rejected`, `memory/bulletins`, and `memory/logs/memory.log` are created after supervisor startup.
- Required action: Add integration assertions in runtime supervisor tests to verify concrete path creation and log file emission for enabled memory worker.

4. Medium: Provisioning path bootstrap has no behavior test coverage.
- Task gap: P14-T01 explicitly includes provisioning behavior.
- Evidence: provisioning hook exists in `save_orchestrator_config` (`src/config/save.rs:48`), but there is no test exercising this path (current config save test is type-surface only at `tests/config_save_module.rs:5`).
- Required action: Add an integration/unit test for `save_orchestrator_config` asserting memory runtime directories are created under the orchestrator private workspace.
