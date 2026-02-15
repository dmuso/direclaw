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

- 2026-02-15 11:35 - Domain modelling doc created
- 2026-02-15 12:22 - Split config typed primitives into `src/config/typed_fields.rs` and re-exported from `config`; added integration coverage for `direclaw::config::typed_fields::*` access path.
