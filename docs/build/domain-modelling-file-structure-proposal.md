## Domain-First Module Restructure for DireClaw

### Summary
Current structure is functionally correct but concentrated in a few very large files (`src/orchestrator.rs`, `src/commands.rs`, `src/runtime.rs`, `src/tui/setup.rs`) that mix multiple domains and execution layers.  
Proposed target is a **domain-first, flat feature module layout** with explicit boundaries for queue, orchestration, runtime, providers, channels, config, and setup.  
Per your preferences, this is a **target-state blueprint only**, with **breaking path renames allowed**, and includes a matching **tests reorganization**.

### Target `src/` File Structure

```text
src/
  lib.rs
  bin/
    direclaw.rs

  app/
    mod.rs
    cli.rs
    command_catalog.rs
    command_dispatch.rs
    command_handlers/
      mod.rs
      daemon.rs
      orchestrators.rs
      workflows.rs
      agents.rs
      channel_profiles.rs
      provider.rs
      channels.rs
      auth.rs
      update.rs
      doctor.rs
      attach.rs

  shared/
    mod.rs
    ids.rs
    errors.rs
    time.rs
    fs_atomic.rs
    serde_ext.rs

  config/
    mod.rs
    paths.rs
    settings.rs
    orchestrators_registry.rs
    orchestrator_file.rs
    load.rs
    save.rs
    validate.rs
    typed_fields.rs
    setup_draft.rs

  queue/
    mod.rs
    message.rs
    paths.rs
    lifecycle.rs
    scheduler.rs
    outbound.rs
    file_tags.rs
    logging.rs

  orchestration/
    mod.rs
    routing.rs
    selector.rs
    selector_artifacts.rs
    function_registry.rs
    workflow_engine.rs
    run_store.rs
    progress.rs
    step_execution.rs
    prompt_render.rs
    output_contract.rs
    transitions.rs
    workspace_access.rs
    diagnostics.rs

  provider/
    mod.rs
    types.rs
    model_map.rs
    prompt_files.rs
    invocation.rs
    output_parse.rs
    runner.rs

  runtime/
    mod.rs
    state_paths.rs
    supervisor.rs
    ownership_lock.rs
    worker_registry.rs
    queue_worker.rs
    channel_worker.rs
    heartbeat_worker.rs
    recovery.rs
    logging.rs

  channels/
    mod.rs
    slack/
      mod.rs
      api.rs
      auth.rs
      ingest.rs
      egress.rs
      cursor_store.rs

  setup/
    mod.rs
    state.rs
    actions.rs
    navigation.rs
    screens.rs
    persistence.rs

  templates/
    mod.rs
    orchestrator_templates.rs
    workflow_step_defaults.rs
```

### Boundary Rules (Decision-Complete)

- `app/*` may depend on all domains; domain modules must not depend on `app/*`.
- `orchestration/*` may depend on `config`, `provider`, `queue`, `shared`; must not depend on `runtime` or `channels`.
- `runtime/*` orchestrates workers and may call `queue`, `orchestration`, `channels`, `provider`, `config`.
- `channels/*` may depend on `queue`, `config`, `shared`; must not depend on `orchestration` internals directly.
- `templates/*` only depends on `config` types and `shared`.
- `setup/*` depends on `config`, `templates`, `shared`, and UI libs; no direct calls into runtime workers.
- All file IO helper logic (`atomic write`, parent-dir sync, canonicalization helpers) lives in `shared/fs_atomic.rs` and is reused.

### Test File Structure (Reorganized to Match Domains)

```text
tests/
  support/
    mod.rs
    fixtures.rs
    scripts.rs
    assertions.rs

  queue_integration.rs
  queue/
    lifecycle.rs
    scheduler.rs
    file_tags.rs

  orchestration_integration.rs
  orchestration/
    selector.rs
    workflow_engine.rs
    output_contract.rs
    workspace_access.rs
    diagnostics.rs

  runtime_integration.rs
  runtime/
    bootstrap.rs
    supervisor.rs
    recovery.rs

  channels_slack_integration.rs
  channels/slack/
    sync.rs
    outbound.rs
    auth.rs

  config_integration.rs
  config/
    typing.rs
    validation.rs
    setup_draft.rs

  provider_integration.rs
  provider/
    runner.rs
    parse.rs

  app_cli_integration.rs
  app_cli/
    command_surface.rs
    auth_sync.rs
    update.rs
    release_gates.rs
```

### Required Test Scenarios

- Queue lifecycle semantics stay atomic: `incoming -> processing -> outgoing`, with requeue safety.
- Per-key scheduler ordering/concurrency remains unchanged.
- Channel-originated messages still route only through orchestration selector path.
- Workflow status commands remain read-only (no step advancement).
- Workspace allowlist/grants continue enforcing deny-by-default behavior.
- Provider invocation behavior unchanged: binary resolution, timeout handling, parse failures, invocation logs.
- Runtime supervisor lifecycle still supports idempotent start/stop/restart and stale-lock recovery.
- Slack sync semantics unchanged across mention filters, channel allowlists, and outbound chunking.
- CLI command surface parity remains spec-aligned after module path breakup.

## Running Log

This is a running log of refactor changes made to iteratively reach the desired structures. Record the date and description of work

- 2026-02-15 11:35 - Created the domain modelling doc.
- 2026-02-15 12:22 - Moved config typed primitives to `src/config/typed_fields.rs`, re-exported from `config`, and added integration coverage.
- 2026-02-15 13:21 - Moved config path/loading helpers to `src/config/paths.rs`, re-exported from `config`, and added integration coverage.
- 2026-02-15 13:24 - Moved settings types/validation to `src/config/settings.rs`, re-exported from `config`, and added integration coverage.
- 2026-02-15 13:28 - Moved orchestrator config types/loading/validation to `src/config/orchestrator_file.rs`, re-exported, and added integration coverage.
- 2026-02-15 13:30 - Moved config loaders to `src/config/load.rs`, updated re-exports, and added integration coverage.
- 2026-02-15 13:39 - Added `src/config/validate.rs`, re-exported validation entry points, and added integration coverage.
- 2026-02-15 13:43 - Moved command catalog to `src/app/command_catalog.rs`, added `src/app/mod.rs`, and added integration coverage.
- 2026-02-15 14:05 - Moved function-invocation planning to `src/app/command_dispatch.rs`, kept compatibility re-exports, and added integration coverage.
- 2026-02-15 13:51 - Moved `update` command handling to `src/app/command_handlers/update.rs` and added integration coverage.
- 2026-02-15 13:54 - Moved CLI parsing/help to `src/app/cli.rs`, kept compatibility re-exports, and added integration coverage.
- 2026-02-15 13:55 - Moved provider/model handlers to `src/app/command_handlers/provider.rs` and added integration coverage.
- 2026-02-15 13:59 - Moved auth handling to `src/app/command_handlers/auth.rs` and added integration coverage.
- 2026-02-15 14:22 - Moved doctor/attach handlers to `src/app/command_handlers/doctor.rs` and `src/app/command_handlers/attach.rs` and added integration coverage.
- 2026-02-15 14:07 - Moved channel-profile handling to `src/app/command_handlers/channel_profiles.rs` and added integration coverage.
- 2026-02-15 14:34 - Moved daemon/supervisor handling to `src/app/command_handlers/daemon.rs` and added integration coverage.
- 2026-02-15 14:15 - Moved orchestrator handling to `src/app/command_handlers/orchestrators.rs` and added integration coverage.
- 2026-02-15 14:52 - Moved workflow handling to `src/app/command_handlers/workflows.rs` and added integration coverage.
- 2026-02-15 14:22 - Moved orchestrator-agent handling to `src/app/command_handlers/agents.rs` and added integration coverage.
- 2026-02-15 14:26 - Moved `send` and `channels` command handling to `src/app/command_handlers/channels.rs`, updated command wiring, and added integration coverage.
- 2026-02-15 14:29 - Added `src/app/command_support.rs` for runtime/config command helpers, updated app/tui handlers to consume it directly, and converted `src/commands.rs` to compatibility re-exports for those helpers.
- 2026-02-15 14:34 - Extracted queue file-tag and outbound normalization logic to `src/queue/file_tags.rs`, re-exported queue helper APIs for compatibility, and added integration coverage for the new module path.
- 2026-02-15 14:36 - Extracted per-key queue scheduler domain logic to `src/queue/scheduler.rs`, re-exported scheduler APIs from `queue` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 14:39 - Extracted queue lifecycle claim/complete/requeue behavior to `src/queue/lifecycle.rs`, re-exported lifecycle APIs from `queue` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 14:41 - Extracted queue path/filename helpers to `src/queue/paths.rs`, re-exported paths APIs from `queue` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 14:44 - Extracted queue message types to `src/queue/message.rs`, re-exported message APIs from `queue` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 14:46 - Extracted outbound message shaping/types/constants to `src/queue/outbound.rs`, kept compatibility exports via `queue` and `queue::file_tags`, and added integration coverage for the new module path.
- 2026-02-15 14:50 - Extracted provider model mapping and invocation planning to `src/provider/model_map.rs` and `src/provider/invocation.rs`, re-exported APIs from `provider` for compatibility, and added integration coverage for the new module paths.
- 2026-02-15 14:52 - Extracted runtime state-root path/bootstrap concerns to `src/runtime/state_paths.rs`, re-exported APIs from `runtime` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 15:02 - Extracted provider output parsing logic to `src/provider/output_parse.rs`, re-exported parser APIs from `provider` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 15:06 - Extracted runtime worker identity/state/registry concerns to `src/runtime/worker_registry.rs`, re-exported APIs from `runtime` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 15:12 - Extracted runtime supervisor state/ownership/lock lifecycle APIs to `src/runtime/supervisor.rs`, re-exported supervisor APIs from `runtime` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 15:16 - Extracted runtime queue startup recovery behavior to `src/runtime/recovery.rs`, re-exported recovery APIs from `runtime` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 15:07 - Extracted provider process runner concerns to `src/provider/runner.rs`, re-exported runner APIs from `provider` for compatibility, and added integration coverage for the new module path.
- 2026-02-15 15:20 - Introduced `src/channels/mod.rs`, moved Slack channel adapter implementation to `src/channels/slack/mod.rs`, kept `src/slack.rs` as compatibility re-exports, and added integration coverage for the new `direclaw::channels::slack` module path.
- 2026-02-15 15:33 - Introduced `src/orchestration/mod.rs`, extracted selector routing schema/validation/retry concerns to `src/orchestration/selector.rs`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::selector` module path.
- 2026-02-15 15:18 - Extracted selector artifact persistence to `src/orchestration/selector_artifacts.rs`, exposed the module via `orchestration`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::selector_artifacts` module path.
- 2026-02-15 15:23 - Extracted workflow run state/store/progress/attempt persistence to `src/orchestration/run_store.rs`, exposed the module via `orchestration`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::run_store` module path.
- 2026-02-15 15:26 - Extracted workspace access context/enforcement/path-normalization logic to `src/orchestration/workspace_access.rs`, exposed the module via `orchestration`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::workspace_access` module path.
- 2026-02-15 17:15 - Extracted step prompt rendering (`StepPromptRender`, placeholder/template resolution, and `render_step_prompt`) to `src/orchestration/prompt_render.rs`, exposed the module via `orchestration`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::prompt_render` module path.
- 2026-02-15 17:21 - Extracted workflow output-contract/result parsing and output-path resolution concerns to `src/orchestration/output_contract.rs`, exposed the module via `orchestration`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::output_contract` module path.
- 2026-02-15 17:49 - Extracted selector-routing function registry and status run-id resolution concerns (`FunctionCall`, `FunctionRegistry`, `StatusResolutionInput`, `resolve_status_run_id`) to `src/orchestration/routing.rs`, exposed the module via `orchestration`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::routing` module path.
- 2026-02-15 17:27 - Introduced `src/orchestration/workflow_engine.rs` and moved execution-safety domain concerns (`ExecutionSafetyLimits`, `resolve_execution_safety_limits`, `enforce_execution_safety`) from `orchestrator`, exposed via `orchestration`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::workflow_engine` module path.
- 2026-02-15 17:31 - Introduced `src/orchestration/diagnostics.rs` and moved orchestration diagnostics/security logging concerns (`append_security_log`, `provider_error_log`, `persist_provider_invocation_log`, `persist_selector_invocation_log`) out of `orchestrator`, exposed via `orchestration`, and added integration coverage for the new `direclaw::orchestration::diagnostics` module path.
- 2026-02-15 17:35 - Introduced `src/orchestration/function_registry.rs` and moved function invocation/catalog concerns (`FunctionCall`, `FunctionRegistry`) out of `orchestration/routing`, exposed via `orchestration`, kept compatibility exports via `orchestration::routing` and `orchestrator`, and added integration coverage for the new `direclaw::orchestration::function_registry` module path.
- 2026-02-15 17:39 - Expanded `src/orchestration/workflow_engine.rs` to own next-step pointer resolution and retryability classification (`NextStepPointer`, `resolve_next_step_pointer`, `is_retryable_step_error`), updated `orchestrator` to consume/re-export these APIs for compatibility, and added integration coverage for the new `direclaw::orchestration::workflow_engine` module surface.
- 2026-02-15 17:44 - Moved `WorkflowEngine` and step-execution helpers (`resolve_runner_binaries`, prompt instruction shaping, latest-step output loading) from `src/orchestrator.rs` to `src/orchestration/workflow_engine.rs`, kept compatibility exports via `orchestrator`, and added integration coverage for constructing the engine from `direclaw::orchestration::workflow_engine::WorkflowEngine`.
- 2026-02-15 17:49 - Introduced `src/orchestration/transitions.rs` and moved selector-action transition routing concerns (`RoutedSelectorAction`, `RouteContext`, `route_selector_action`, plus selector-run input/output-path validation helpers) out of `src/orchestrator.rs`, exposed via `orchestration`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::transitions` module path.
