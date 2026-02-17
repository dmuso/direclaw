# Phase 16 Review: Action Required

## Findings

1. Critical: Heartbeat queue messages are not routable through orchestrator processing as implemented.
- Spec mismatch: heartbeat traffic must use the same queue contract/processing guarantees as other messages (`docs/build/spec/11-heartbeat-service.md:29`).
- Current implementation builds heartbeat messages with `channel_profile_id: None` (`src/runtime/heartbeat_worker.rs:53`), but queued message processing always resolves orchestrator via `channel_profile_id` and fails if missing (`src/orchestration/routing.rs:121`, `src/orchestration/selector.rs:20`).
- Impact: once queue worker claims heartbeat messages, they fail orchestrator resolution and are requeued/fail-looped instead of executing end-to-end.
- Action: add a deterministic orchestrator-resolution path for heartbeat queue items (for example, routing by queue scope/orchestrator context), and add an integration test that drives heartbeat message through `incoming -> processing -> outgoing`.

2. High: Response matching logic is too strict to be observable under normal runtime timing.
- Spec requirement: matching heartbeat responses should be observable in logs (`docs/build/spec/11-heartbeat-service.md:23`, `docs/build/spec/11-heartbeat-service.md:37`).
- Current matcher requires exact `message_id` equality (`src/runtime/heartbeat_worker.rs:83`) and is called immediately after enqueue in the same tick (`src/runtime/heartbeat_worker.rs:193`).
- Impact: in normal operation, queue processing happens asynchronously after enqueue; by the time a response exists, next tick usually has a different `message_id`, so matched logs are effectively unreachable outside test-only timestamp forcing (`tests/runtime_heartbeat_worker_module.rs:320`).
- Action: correlate on stable heartbeat correlation metadata across ticks (not just same-tick `message_id`), or persist pending heartbeat ids and evaluate responses on later ticks. Add an integration test with real queue processing timing (no env timestamp override) that produces `heartbeat.response.matched`.

3. Medium: Heartbeat interval default behavior does not match spec default.
- Spec states default interval is `3600` seconds (`docs/build/spec/11-heartbeat-service.md:10`).
- Current implementation treats missing `monitoring.heartbeat_interval` as disabled (`None`) via `unwrap_or(0)` (`src/runtime/heartbeat_worker.rs:10`).
- Impact: heartbeat worker does not start unless interval is explicitly set, contrary to spec default semantics.
- Action: make missing interval default to `3600` and add/update tests for implicit-default enablement.
