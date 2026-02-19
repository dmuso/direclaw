# DireClaw Test Performance Report

- Run timestamp: 2026-02-20 08:35:19 AEDT
- Environment: `nix-shell`
- Scope: exhaustive per-test timing sweep across all compiled test binaries
- Total timed test executions: 489
- Threshold: `> 2.0s`
- Over-threshold entries: 7

## Method
1. Compiled tests once with `cargo test --all --no-run --message-format=json`.
2. Enumerated each test from each compiled binary via `--list --format terse`.
3. Ran each test in exact mode (`--exact`) and measured wall-clock runtime.
4. Sorted results descending by runtime and filtered entries where runtime exceeded 2.0 seconds.

## Tests Over 2.0 Seconds

| Runtime (s) | Status | Binary | Test |
|---:|---:|---|---|
| 2.247 | 0 | `runtime_supervisor-4651dd7c7801cea1` | `slow_shutdown_fault_injection_reports_timeout_state_and_log` |
| 2.208 | 0 | `message_flow_queue_orchestrator_provider_e2e-6d45e78f245f40cc` | `queue_runtime_enforces_same_key_ordering_and_cross_key_concurrency` |
| 2.170 | 0 | `runtime_supervisor-4651dd7c7801cea1` | `repeated_start_status_restart_never_corrupts_runtime_state` |
| 2.069 | 0 | `orchestrator_workflow_engine-79ed6552e105e2fc` | `run_timeout_uses_elapsed_runtime_across_multiple_steps` |
| 2.064 | 0 | `message_flow_queue_orchestrator_provider_e2e-6d45e78f245f40cc` | `provider_timeout_is_logged_and_falls_back_deterministically` |
| 2.034 | 0 | `cli_command_surface-fb8d8e89f255c6cf` | `workflow_run_enforces_step_timeout_from_cli_config` |
| 2.018 | 0 | `provider_runner-85e4fd750bff739c` | `provider_timeout_is_explicit` |

## Artifacts
- Raw timings: `docs/build/review/slow-tests/direclaw-test-times.tsv`
- Sorted timings: `docs/build/review/slow-tests/direclaw-test-times-sorted.tsv`
