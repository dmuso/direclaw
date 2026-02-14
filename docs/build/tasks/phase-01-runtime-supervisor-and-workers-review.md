# Phase 01 Review: Runtime Supervisor and Worker Lifecycle

## Scope Reviewed
- Uncommitted changes in:
  - `src/runtime.rs`
  - `src/cli.rs`
  - `tests/runtime_supervisor.rs`
  - `tests/cli_command_surface.rs`
  - `docs/build/tasks/phase-01-runtime-supervisor-and-workers.md`
- Requirements baseline:
  - `docs/build/release-readiness-plan.md` (Phase 01 + lifecycle/health expectations)

## Findings Requiring Action

1. **Critical: runtime state writes are non-atomic and currently cause user-visible command/test failures.**
- Evidence:
  - `save_supervisor_state` writes directly with `fs::write` in `src/runtime.rs:305`.
  - `cmd_start` and supervisor both write `daemon/runtime.json` during startup (`src/cli.rs:278`, `src/runtime.rs:521`).
  - Reproduced failures:
    - `nix-shell --run 'cargo test --test runtime_supervisor --test cli_command_surface'` failed with `EOF while parsing a value` for `daemon/runtime.json`.
    - `nix-shell --run 'cargo test --all'` failed in `daemon_command_surface_works` with same parse error.
- Why this blocks Phase 01:
  - Conflicts with “graceful start/stop/restart semantics and health reporting” and “persist runtime state for observability” in `docs/build/release-readiness-plan.md:96` and `docs/build/release-readiness-plan.md:97`.
- Action:
  - Make state persistence atomic (temp file + fsync + rename), and avoid concurrent unsynchronized writers to the same file during startup.
  - Add regression test that stress-loops `start/status/restart` and asserts no parse errors.

2. **High: `status` reports persisted flags, not verified live supervisor state, so health can be stale/incorrect.**
- Evidence:
  - `cmd_status` only reads `runtime.json` and prints fields (`src/cli.rs:323`).
  - It does not reconcile with `supervisor_ownership_state` / PID liveness before reporting (`src/cli.rs:323` vs `src/runtime.rs:352`).
- Why this blocks Phase 01:
  - Phase 01 requires actual health reporting, not simulated/stale state (`docs/build/release-readiness-plan.md:64`, `docs/build/release-readiness-plan.md:96`).
- Action:
  - In `status`, verify ownership/liveness and either self-heal stale state or report explicit degraded/stale runtime status.

3. **High: `stop` can report success even if process remains alive after forced kill attempts.**
- Evidence:
  - `stop_active_supervisor` sends `-TERM`, then `-KILL`, then unconditionally calls `cleanup_stale_supervisor` and returns `Ok` (`src/runtime.rs:478`, `src/runtime.rs:493`).
- Why this blocks Phase 01:
  - Violates expectation that `stop` exits cleanly and leaves no orphans (`docs/build/tasks/phase-01-runtime-supervisor-and-workers.md:52`).
- Action:
  - Re-check liveness after kill escalation; if still alive, return an error and keep state reflecting failure instead of forcing `running=false`.

4. **Medium: task marked `complete` but required Phase 01 tests are not fully implemented.**
- Evidence:
  - Task file marks all items complete (`docs/build/tasks/phase-01-runtime-supervisor-and-workers.md:16`, `docs/build/tasks/phase-01-runtime-supervisor-and-workers.md:32`, `docs/build/tasks/phase-01-runtime-supervisor-and-workers.md:46`, `docs/build/tasks/phase-01-runtime-supervisor-and-workers.md:60`).
  - Missing required coverage in same file:
    - Slow-worker shutdown fault-injection test (`docs/build/tasks/phase-01-runtime-supervisor-and-workers.md:56`).
    - Snapshot-style status/log assertions (`docs/build/tasks/phase-01-runtime-supervisor-and-workers.md:68`).
  - Current runtime tests (`tests/runtime_supervisor.rs:91`) cover start/stop/restart and injected worker error, but not slow-shutdown timeout behavior or status/log snapshot stability.
- Why this blocks Phase 01:
  - Requirement traceability and acceptance evidence are incomplete relative to the declared `complete` status.
- Action:
  - Add missing tests and only keep statuses as `complete` once they pass reliably.

## Validation Run
- `nix-shell --run 'cargo test --test runtime_supervisor --test cli_command_surface'` -> **failed** (runtime state parse EOF).
- `nix-shell --run 'cargo test --all'` -> **failed** (same runtime state parse EOF in CLI surface tests).
