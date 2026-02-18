# Phase 17 Scheduler/Cron Core Review

## Findings

### 1) Critical: overlap guard permanently blocks recurring jobs
- Spec refs: `docs/build/spec/15-scheduled-automation.md:161`, `docs/build/spec/15-scheduled-automation.md:203`
- Code refs: `src/orchestration/scheduler.rs:375`, `src/orchestration/scheduler.rs:407`
- Problem: overlap suppression is implemented as `last_result == "dispatched"`, and `last_result` is set to `"dispatched"` after each trigger with no completion signal to clear it. For `interval`/`cron` jobs with `allow_overlap=false`, this causes one dispatch and then permanent suppression.
- Action needed:
1. Replace `last_result`-based overlap checks with active-execution tracking keyed by `job_id`/`execution_id` lifecycle.
2. Add regression tests proving recurring jobs continue to run when prior executions are complete, while true overlaps are blocked.

### 2) High: misfire recovery computes `nextRunAt` from stale timestamps, causing catch-up replay/skip loops
- Spec refs: `docs/build/spec/15-scheduled-automation.md:155`, `docs/build/spec/15-scheduled-automation.md:159`, `docs/build/spec/15-scheduled-automation.md:206`
- Code refs: `src/orchestration/scheduler.rs:360`, `src/orchestration/scheduler.rs:361`, `src/orchestration/scheduler.rs:408`, `src/orchestration/scheduler.rs:634`, `src/orchestration/scheduler.rs:651`
- Problem: when a run is overdue, next run is computed from old `next_run_at` rather than from recovery time. For long downtime, this can emit a backlog of immediate triggers (`fire_once_on_recovery`) or repeated skip ticks (`skip_missed`) instead of advancing directly to the next future slot.
- Action needed:
1. For recovery logic, advance schedule to the first future run relative to `now` after applying misfire policy.
2. Add integration tests that simulate long downtime for `interval` and `cron` schedules for both policies.

### 3) High: scheduled `workflow_start.inputs` payload is dropped
- Spec refs: `docs/build/spec/15-scheduled-automation.md:63`, `docs/build/spec/15-scheduled-automation.md:66`
- Code refs: `src/orchestration/routing.rs:507`
- Problem: `TargetAction::WorkflowStart { workflow_id, .. }` ignores `inputs`; only `workflow_id` is routed. Scheduled workflow inputs therefore never reach run initialization.
- Action needed:
1. Extend routing/run-start path to carry scheduler-provided workflow inputs.
2. Add integration test asserting scheduled workflow inputs appear in run start inputs/progress artifacts.

### 4) High: `schedule.show/update/pause/resume/delete/run_now` are not orchestrator-scoped
- Spec refs: `docs/build/spec/15-scheduled-automation.md:114`, `docs/build/spec/15-scheduled-automation.md:128`
- Code refs: `src/app/command_dispatch.rs:484`
- Problem: job lookup scans all orchestrators and uses the first match. A command executed in one orchestrator scope can operate on another orchestrator's job if the `jobId` is known.
- Action needed:
1. Bind job operations to resolved orchestrator scope (or require `orchestratorId` for mutating operations).
2. Add tests proving cross-orchestrator job access is rejected.

### 5) Medium: required scheduler audit/observability events are largely missing
- Spec refs: `docs/build/spec/15-scheduled-automation.md:184`, `docs/build/spec/15-scheduled-automation.md:188`
- Code refs: `src/runtime/scheduler_worker.rs:21`, `src/orchestration/scheduler.rs:345`, `src/app/command_dispatch.rs:320`
- Problem: current logging records a periodic `scheduler.tick` summary, but not required lifecycle events (create/update/delete/pause/resume/trigger/misfire actions).
- Action needed:
1. Emit structured runtime/audit logs for each required scheduler lifecycle and trigger event.
2. Add tests asserting event emission for at least create, pause/resume, trigger dispatch/failure, and misfire handling.

## Validation run
- Executed scheduler-focused tests in `nix-shell`:
1. `cargo test --test scheduler_domain_module --test scheduler_worker_module --test scheduler_routing_integration_module --test scheduler_command_surface_module`
- Result: all passed, which indicates current tests do not yet cover the failure modes above.
