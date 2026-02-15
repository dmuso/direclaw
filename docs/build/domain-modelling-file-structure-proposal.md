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
- 2026-02-15 13:21 - Extracted config path/loading helpers into `src/config/paths.rs` (`GLOBAL_*`, `default_*_config_path`, `load_global_settings`) and re-exported from `config`; added integration coverage for `direclaw::config::paths::*` access path.
- 2026-02-15 13:24 - Extracted settings-domain types and validation from `src/config.rs` into `src/config/settings.rs` (`Settings*`, channel/auth config structs, `ValidationOptions`, and `Settings` impl); re-exported from `config` and added integration coverage for `direclaw::config::settings::*` access path.
- 2026-02-15 13:28 - Extracted orchestrator-file domain types/loading/validation from `src/config.rs` into `src/config/orchestrator_file.rs` (`OrchestratorConfig`, workflow/agent structs, orchestration limits, `load_orchestrator_config`, and related enums/helpers); re-exported from `config` and added integration coverage for `direclaw::config::orchestrator_file::*` access path.
- 2026-02-15 13:30 - Extracted config loader functions into new `src/config/load.rs` (`load_global_settings`, `load_orchestrator_config`) to align with target `config/load.rs`; updated root re-exports and added integration coverage for `direclaw::config::load::*` access path.
- 2026-02-15 13:39 - Added new `src/config/validate.rs` with typed validation entry points (`validate_settings`, `validate_orchestrator_config`) and re-exported them from `config`; added integration coverage for `direclaw::config::validate::*` access path.
- 2026-02-15 13:43 - Split command metadata catalog out of `src/commands.rs` into `src/app/command_catalog.rs` and introduced `src/app/mod.rs`; re-exported catalog symbols from `commands` for compatibility, exported new `direclaw::app` module, and added integration coverage for `direclaw::app::command_catalog::*` access path.
- 2026-02-15 14:05 - Extracted selector invocation planning into `src/app/command_dispatch.rs` (`InternalFunction`, `FunctionExecutionPlan`, `plan_function_invocation`) and exported it via `app`; kept `commands` compatibility by re-exporting the moved symbols and added integration coverage for `direclaw::app::command_dispatch::*` access path.
- 2026-02-15 13:51 - Extracted update command handling from `src/commands.rs` into `src/app/command_handlers/update.rs` (`cmd_update`, release metadata parsing, version comparison, and update-check HTTP logic), introduced `src/app/command_handlers/mod.rs`, wired `app::mod` exports, delegated CLI `update` verb through the new handler, and added integration coverage for `direclaw::app::command_handlers::update::cmd_update`.
- 2026-02-15 13:54 - Extracted CLI command-surface parsing/help rendering from `src/commands.rs` into `src/app/cli.rs` (`CliVerb`, `parse_cli_verb`, `cli_help_lines`, selector help, and shared help text), exported the new module via `app`, preserved `commands` compatibility through re-exports, and added integration coverage for `direclaw::app::cli::*` access path.
- 2026-02-15 13:55 - Extracted provider/model CLI handlers from `src/commands.rs` into `src/app/command_handlers/provider.rs` (`cmd_provider`, `cmd_model`), exported the handler module via `app::command_handlers`, wired `run_cli` delegation through the new module, and added integration coverage for `direclaw::app::command_handlers::provider::*` access path.
- 2026-02-15 13:59 - Extracted auth command handling and auth-sync file execution flow from `src/commands.rs` into `src/app/command_handlers/auth.rs` (`cmd_auth`, auth-sync result rendering, source sync backend dispatch, and OnePassword destination write/permission logic), exported the handler via `app::command_handlers`, delegated CLI `auth` through the new module, and added integration coverage for `direclaw::app::command_handlers::auth::cmd_auth`.
- 2026-02-15 14:22 - Extracted doctor/attach command handling from `src/commands.rs` into `src/app/command_handlers/doctor.rs` (`cmd_doctor`, doctor diagnostics helpers) and `src/app/command_handlers/attach.rs` (`cmd_attach`), exported both via `app::command_handlers`, delegated CLI `doctor`/`attach` through the new handlers, and added integration coverage for `direclaw::app::command_handlers::doctor::cmd_doctor` and `direclaw::app::command_handlers::attach::cmd_attach`.
