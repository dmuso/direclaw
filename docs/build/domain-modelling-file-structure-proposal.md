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
- 2026-02-15 17:55 - Introduced `src/orchestration/progress.rs` and moved workflow progress snapshot typing (`ProgressSnapshot`) out of `src/orchestration/run_store.rs`, exposed via `orchestration`, kept compatibility exports via `run_store`, and added integration coverage for the new `direclaw::orchestration::progress` module path.
- 2026-02-15 18:02 - Introduced `src/orchestration/step_execution.rs` and moved workflow step-attempt execution concerns (workspace resolution/enforcement, prompt artifact writing, provider invocation, and latest-step output loading) out of `src/orchestration/workflow_engine.rs`; exposed via `orchestration`, kept compatibility export of `resolve_runner_binaries` via `workflow_engine`, and added integration coverage for the new `direclaw::orchestration::step_execution` module path.
- 2026-02-15 18:06 - Moved selector provider-attempt execution (`run_selector_attempt_with_provider`) from `src/orchestrator.rs` to `src/orchestration/selector.rs`, kept compatibility export via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::selector::run_selector_attempt_with_provider` module path.
- 2026-02-15 18:21 - Introduced `src/queue/logging.rs` and moved queue security-log appends (`append_queue_log`) out of `src/queue/lifecycle.rs`; exposed via `queue::logging`, updated lifecycle to consume the extracted helper, and added integration coverage for the new `direclaw::queue::logging` module path.
- 2026-02-15 18:11 - Introduced `src/runtime/logging.rs` and moved runtime log appends (`append_runtime_log`) out of `src/runtime.rs`; exposed via `runtime::logging`, kept compatibility re-export via `runtime`, and added integration coverage for the new `direclaw::runtime::logging` module path.
- 2026-02-15 18:36 - Introduced `src/runtime/queue_worker.rs` and moved queue worker execution concerns (`drain_queue_once`, `drain_queue_once_with_binaries`, queue processor loop, claimed-message processing, outbound action shaping, and runner-binary resolution) out of `src/runtime.rs`; exposed via `runtime::queue_worker`, kept compatibility re-exports via `runtime`, and added integration coverage for the new `direclaw::runtime::queue_worker` module path.
- 2026-02-16 10:04 - Moved scripted setup key parsing from `src/setup/actions.rs` to `src/setup/navigation.rs` (`parse_scripted_setup_keys`) to group setup key interpretation in one module, updated setup actions to consume the new helper, and added integration coverage in `tests/setup_navigation_module.rs`.
- 2026-02-15 18:48 - Added `src/shared/serde_ext.rs` to match the target shared module layout, extracted generic string-backed deserialization parsing (`parse_via_string`) from duplicated call sites, updated `shared::ids` and `config::typed_fields` to consume it, and added integration coverage for the new `direclaw::shared::serde_ext` module path.
- 2026-02-16 09:09 - Updated runtime worker code to depend on `crate::channels::slack` directly instead of the `src/slack.rs` compatibility shim, and added boundary coverage in `tests/runtime_boundary_module.rs` to prevent future runtime imports of `use crate::slack`.
- 2026-02-15 18:48 - Updated `src/runtime/queue_worker.rs` to depend directly on `src/orchestration/*` modules (`function_registry`, `routing`, `run_store`, `selector`, `transitions`) instead of `crate::orchestrator` compatibility exports, and added `tests/runtime_boundary_module.rs` to enforce the runtime-to-orchestration boundary going forward.
- 2026-02-15 18:44 - Moved runtime polling defaults (`PollingDefaults`) from `src/runtime/mod.rs` to `src/runtime/channel_worker.rs`, exposed the type via `runtime::channel_worker` while keeping compatibility re-export via `runtime`, and added integration coverage for the new `direclaw::runtime::channel_worker::PollingDefaults` module path.
- 2026-02-15 20:06 - Introduced `src/shared/time.rs` and moved runtime unix-timestamp helper ownership to `shared` (`now_secs`), re-exported for runtime-internal compatibility, and added integration coverage for the new `direclaw::shared::time` module path.
- 2026-02-15 19:49 - Introduced `src/orchestration/error.rs` and moved `OrchestratorError` plus config-error conversion out of `src/orchestrator.rs`; exposed via `orchestration::error`, kept compatibility export via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::error` module path.
- 2026-02-15 19:01 - Introduced `src/provider/types.rs` and moved provider domain typing/error concerns (`ProviderError`, `ProviderKind`, `PromptArtifacts`, `ProviderRequest`, `InvocationSpec`, `InvocationLog`, `ProviderResult`, and `io_error`) out of `src/provider.rs`; exposed via `provider::types`, kept compatibility re-exports via `provider`, and added integration coverage for the new `direclaw::provider::types` module path.
- 2026-02-15 18:50 - Introduced `src/runtime/heartbeat_worker.rs` and moved heartbeat tick behavior behind `tick_heartbeat_worker`, updated `channel_worker` to call the extracted module, exposed via `runtime::heartbeat_worker`, and added integration coverage for the new `direclaw::runtime::heartbeat_worker` module path.
- 2026-02-15 18:40 - Moved supervisor loop orchestration (`run_supervisor` and worker-event state application) from `src/runtime.rs` to `src/runtime/supervisor.rs`, exposed the loop entrypoint via `runtime::supervisor::run_supervisor` while keeping compatibility re-exports via `runtime`, added integration coverage for the new module path, and stabilized runtime tests by serializing `HOME` env mutation in `default_state_root_path_uses_home_direclaw`.
- 2026-02-15 18:44 - Introduced `src/channels/slack/cursor_store.rs` and moved Slack cursor state persistence concerns (`SlackCursorState`, `load_cursor_state`, `save_cursor_state`) out of `src/channels/slack/mod.rs`; exposed via `channels::slack::cursor_store`, updated Slack sync to consume the extracted module, and added integration coverage for the new `direclaw::channels::slack::cursor_store` module path.
- 2026-02-15 18:50 - Introduced `src/runtime/channel_worker.rs` and moved Slack channel tick execution (`tick_slack_worker`) out of `src/runtime.rs`; updated runtime worker dispatch to consume the extracted helper and added integration coverage for the new `direclaw::runtime::channel_worker` module path.
- 2026-02-15 19:02 - Expanded `src/runtime/channel_worker.rs` to own worker-spec construction and worker execution loop concerns (`WorkerRuntime`, `WorkerSpec`, `WorkerRunContext`, `build_worker_specs`, and `run_worker`), updated `src/runtime.rs` to delegate worker startup/execution to `channel_worker`, and added coverage for the extracted worker-spec builder behavior.
- 2026-02-15 19:10 - Introduced `src/runtime/ownership_lock.rs` and moved supervisor ownership/lock/process-control concerns (`OwnershipState`, `StopResult`, ownership detection, lock reservation/cleanup, supervisor spawn/stop signaling, and process-liveness checks) out of `src/runtime/supervisor.rs`; exposed via `runtime::ownership_lock`, kept compatibility exports via `runtime` and `runtime::supervisor`, and added integration coverage for the new `direclaw::runtime::ownership_lock` module path.
- 2026-02-15 19:24 - Moved queue-message orchestration entrypoints (`process_queued_message` and `process_queued_message_with_runner_binaries`) from `src/orchestrator.rs` to `src/orchestration/routing.rs`, kept compatibility exports via `orchestrator`, and added integration coverage for the new `direclaw::orchestration::routing::process_queued_message` module path.
- 2026-02-15 19:40 - Introduced `src/provider/prompt_files.rs` and moved provider prompt/reset file concerns (`ResetResolution`, `consume_reset_flag`, `write_file_backed_prompt`, `read_to_string`) out of `src/provider.rs`; exposed via `provider::prompt_files`, kept compatibility re-exports via `provider`, and added integration coverage for the new `direclaw::provider::prompt_files` module path.
- 2026-02-15 19:50 - Introduced `src/channels/slack/api.rs` and moved Slack API client/request/response concerns (`SlackApiClient`, request helpers, API envelope/data types, conversation listing/history, and message posting) out of `src/channels/slack/mod.rs`; exposed via `channels::slack::api`, updated Slack sync to consume the extracted module, and added integration coverage for the new `direclaw::channels::slack::api` module path.
- 2026-02-15 19:16 - Moved `ChannelKind` from `src/config.rs` into `src/config/settings.rs`, re-exported it from `config` for compatibility, and added integration coverage for the new `direclaw::config::settings::ChannelKind` module path.
- 2026-02-15 19:19 - Introduced `src/shared/mod.rs` and `src/shared/fs_atomic.rs`, moved atomic file-write and canonicalization helpers (`atomic_write_file`, `canonicalize_existing`) out of `src/runtime.rs`, kept runtime compatibility via re-export/consumption, and added integration coverage for the new `direclaw::shared::fs_atomic` module path.
- 2026-02-15 19:35 - Introduced `src/channels/slack/auth.rs` and moved Slack credential/env-token and profile health/validation concerns (`load_env_config`, `slack_profiles`, `configured_slack_allowlist`, `validate_startup_credentials`, `profile_credential_health`) out of `src/channels/slack/mod.rs`; exposed via `channels::slack::auth`, kept compatibility re-exports via `channels::slack`, and added integration coverage for the new `direclaw::channels::slack::auth` module path.
- 2026-02-15 20:07 - Introduced `src/channels/slack/egress.rs` and moved Slack outbound delivery concerns (`process_outbound`, outgoing-queue file sorting, conversation-id parsing, outbound chunking, and channel-profile resolution) out of `src/channels/slack/mod.rs`; updated Slack sync to delegate outbound delivery to the extracted module and moved parser/chunking unit coverage into `channels::slack::egress`.
- 2026-02-15 19:44 - Introduced `src/channels/slack/ingest.rs` and moved Slack inbound ingestion concerns (message filtering, incoming queue payload shaping, cursor-aware per-profile history ingestion, and queue enqueue writes) out of `src/channels/slack/mod.rs`; updated Slack sync to delegate inbound processing to the extracted module and added integration coverage for the new `direclaw::channels::slack::ingest` module path.
- 2026-02-15 19:53 - Introduced `src/templates/mod.rs`, `src/templates/orchestrator_templates.rs`, and `src/templates/workflow_step_defaults.rs`; moved workflow-template construction and default step prompt/output-contract logic out of `src/workflow.rs`, kept compatibility exports via `workflow`, exposed `templates` from `lib`, and added integration coverage for the new `direclaw::templates::*` module paths.
- 2026-02-15 20:08 - Introduced `src/config/save.rs` and moved config persistence concerns (`save_settings`, `save_orchestrator_config`, `save_orchestrator_registry`, `remove_orchestrator_config`) out of `src/app/command_support.rs`; exposed via `config::save`, re-exported via `config`, kept compatibility wrappers in `app::command_support`, and added integration coverage for the new `direclaw::config::save` module path.
- 2026-02-15 20:15 - Introduced `src/config/orchestrators_registry.rs` and moved registry-scoped orchestrator persistence concerns (`save_orchestrator_registry`, `remove_orchestrator_config`) out of `src/config/save.rs`; exposed via `config::orchestrators_registry`, kept compatibility exports via `config` and `config::save`, and added integration coverage for the new `direclaw::config::orchestrators_registry` module path.
- 2026-02-15 20:09 - Updated orchestration domain modules to import `OrchestratorError` from `src/orchestration/error.rs` directly instead of via the `src/orchestrator.rs` compatibility module, and added boundary coverage in `tests/orchestration_boundary_module.rs` to prevent regressions.
- 2026-02-15 20:13 - Moved the runtime module root file from `src/runtime.rs` to `src/runtime/mod.rs` to align with the target domain directory layout while preserving the existing `direclaw::runtime::*` module surface and compatibility re-exports.
- 2026-02-15 20:28 - Introduced `src/setup/mod.rs` and `src/setup/navigation.rs`, moved setup navigation domain concerns (`SetupScreen`, `SetupAction`, `NavState`, key-to-action mapping, transition routing, and screen item counts) out of `src/tui/setup.rs`, updated TUI setup flow to consume the extracted module, exposed `setup` from `lib`, and added integration coverage for the new `direclaw::setup::navigation` module path.
- 2026-02-15 21:10 - Introduced `src/setup/actions.rs`, `src/setup/state.rs`, `src/setup/screens.rs`, and `src/setup/persistence.rs`; moved setup command flow/state helpers/bootstrap persistence concerns out of `src/tui/setup.rs`, reduced `tui::setup` to a compatibility shim delegating to `setup::actions::cmd_setup`, and preserved setup unit/integration coverage against the extracted module paths.
- 2026-02-16 07:41 - Continued setup-domain separation by moving setup screen rendering/view helpers (`SetupFieldRow`, `field_row`, `tail_for_display`, `centered_rect`, `draw_setup_ui`, `draw_list_screen`, `draw_field_screen`) from `src/setup/actions.rs` into `src/setup/screens.rs`, updated actions to consume the new screen module boundary, and added integration coverage in `tests/setup_screens_module.rs` for the new `direclaw::setup::screens` module path.
- 2026-02-16 07:58 - Moved the config module root from `src/config.rs` to `src/config/mod.rs` to align with the target domain directory layout while preserving the existing `direclaw::config::*` module surface, and added `tests/config_module_layout.rs` coverage to guard the directory-module structure.
- 2026-02-16 08:05 - Expanded `src/app/command_dispatch.rs` to own function-invocation execution concerns (`FunctionExecutionContext`, `execute_function_invocation_with_executor`, `execute_internal_function`, and missing-run remapping), reduced `src/commands.rs` to compatibility wrappers/re-exports for these APIs, and added integration coverage in `tests/app_command_dispatch_module.rs` for the new `direclaw::app::command_dispatch` execution path.
- 2026-02-15 21:24 - Moved the provider module root from `src/provider.rs` to `src/provider/mod.rs` to align with the target domain directory layout while preserving the existing `direclaw::provider::*` module surface, and added `tests/provider_module_layout.rs` coverage to guard the directory-module structure.
- 2026-02-16 08:56 - Moved the queue module root from `src/queue.rs` to `src/queue/mod.rs` to align with the target domain directory layout while preserving the existing `direclaw::queue::*` module surface, and added `tests/queue_module_layout.rs` coverage to guard the directory-module structure.
- 2026-02-16 09:00 - Introduced `src/shared/errors.rs` and moved runtime error typing (`RuntimeError`) out of `src/runtime/mod.rs`; exposed via `shared::errors`, kept compatibility re-export via `runtime`, and added integration coverage for the new `direclaw::shared::errors::RuntimeError` module path.
- 2026-02-16 09:03 - Moved CLI/function invocation entrypoints (`run_cli`, `execute_function_invocation`) from `src/commands.rs` to `src/app/command_handlers/mod.rs`, converted `commands` to compatibility re-exports for those entrypoints, and added integration coverage for the new `direclaw::app::command_handlers::{run_cli, execute_function_invocation}` module path.
- 2026-02-16 09:16 - Introduced `src/runtime/worker_primitives.rs` and moved shared runtime worker primitives (`WorkerEvent`, queue polling/backoff constants, and stop-aware sleep helper) out of `src/runtime/mod.rs`; exposed queue polling defaults via `runtime::worker_primitives`, kept compatibility imports through `runtime`, and added integration coverage for the new `direclaw::runtime::worker_primitives` module path.
- 2026-02-16 09:19 - Added `src/shared/ids.rs` and moved shared identifier wrappers/validation (`OrchestratorId`, `WorkflowId`, `StepId`, `AgentId`, `validate_identifier_value`) out of `src/config/typed_fields.rs`; updated typed-fields to consume/re-export shared IDs for compatibility, exposed the module via `shared`, and added integration coverage for the new `direclaw::shared::ids` module path.
- 2026-02-16 09:32 - Removed the legacy `src/slack.rs` compatibility facade and switched app/tests to the domain-first `channels::slack` module path (`crate::channels::slack`, `direclaw::channels::slack`); dropped `pub mod slack` from `lib.rs` and added boundary coverage in `tests/channels_boundary_module.rs` to prevent reintroducing the legacy module.
- 2026-02-16 09:39 - Extracted setup workflow-edit helper functions (`workflow_inputs_as_csv`, CSV parsing, output-file mapping parse/render, and `unique_step_id`) from `src/setup/actions.rs` into `src/setup/state.rs`, updated setup actions to consume the new state helpers, and added unit coverage in `setup::state::tests` for the extracted behavior.
- 2026-02-16 09:27 - Removed the legacy `src/workflow.rs` compatibility facade and switched app/config/setup imports to the domain-first `templates` module paths (`crate::templates::orchestrator_templates`, `crate::templates::workflow_step_defaults`); dropped `pub mod workflow` from `lib.rs` and added boundary coverage in `tests/templates_boundary_module.rs` to prevent reintroducing the legacy module path.
- 2026-02-15 20:34 - Removed the legacy root `src/cli.rs` compatibility module, updated `src/bin/direclaw.rs` to execute through `app::command_handlers::run_cli`, dropped `pub mod cli` from `src/lib.rs`, and added layout coverage in `tests/library_module_layout.rs` to guard against reintroducing the root CLI facade.
- 2026-02-15 20:41 - Updated app-layer orchestration dependencies to use domain-first modules directly (`orchestration::error`, `orchestration::run_store`, `orchestration::workflow_engine`, `orchestration::workspace_access`) instead of the `src/orchestrator.rs` compatibility facade, and added `tests/app_boundary_module.rs` to prevent future `crate::orchestrator::*` imports from `src/app/*`.
- 2026-02-16 10:02 - Removed the legacy `src/orchestrator.rs` compatibility facade and dropped `pub mod orchestrator` from `src/lib.rs`; migrated remaining integration tests to direct `direclaw::orchestration::*` module paths and added a layout guard in `tests/library_module_layout.rs` to prevent reintroducing the root orchestrator facade.
- 2026-02-15 18:58 - Updated `src/app/command_handlers/mod.rs` to invoke setup via `crate::setup::actions::cmd_setup()` (instead of `crate::tui::setup`), added `app_sources_do_not_depend_on_tui_compat_module` coverage in `tests/app_boundary_module.rs`, and kept `src/tui/setup.rs` as a public compatibility shim.
- 2026-02-16 10:06 - Removed `src/runtime/worker_primitives.rs` (not part of the target runtime structure), moved queue polling defaults/types/constants ownership to `src/runtime/queue_worker.rs` (`queue_polling_defaults`, `QueuePollingDefaults`, `QUEUE_*`), inlined runtime-internal `WorkerEvent` and stop-aware sleep primitives into `src/runtime/mod.rs`, updated supervisor wiring to use `queue_worker::QUEUE_MAX_CONCURRENCY`, and updated integration coverage to validate the `direclaw::runtime::queue_worker` module path.
- 2026-02-16 10:10 - Removed the legacy `src/commands.rs` compatibility facade and dropped `pub mod commands` from `src/lib.rs`; updated orchestration function dispatch to consume `app` domain modules directly (`app::command_catalog::V1_FUNCTIONS`, `app::command_handlers::execute_function_invocation`, `app::command_dispatch::FunctionExecutionContext`) and added a layout guard in `tests/library_module_layout.rs` to prevent reintroducing the root commands module.
