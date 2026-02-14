# Release Requirement Traceability Index

Canonical plan source:
- `docs/build/release-readiness-plan.md`

Table schema:
- `requirement_id`: stable identifier for a release go/no-go requirement
- `plan_requirement`: exact release-blocking requirement text
- `owning_tasks`: phase task ids that deliver the requirement
- `test_references`: current or planned automated test IDs/files enforcing the requirement

| requirement_id | plan_requirement | owning_tasks | test_references |
|---|---|---|---|
| RB-1 | Runtime workers execute continuously under daemon lifecycle. | P01-T01, P01-T02, P01-T03 | `tests/cli_command_surface.rs::daemon_command_surface_works`; `tests/runtime_bootstrap.rs::bootstrapping_empty_state_root_creates_full_tree` |
| RB-2 | Slack end-to-end flow is automated and verified in tests. | P03-T01, P03-T02, P03-T03, P03-T04 | `tests/slack_channel_sync.rs::sync_queues_inbound_and_sends_outbound`; `tests/slack_channel_sync.rs::sync_pages_conversation_history_before_advancing_cursor` |
| RB-3 | Queue/orchestrator/provider pipeline runs in production path (not simulation-only behavior). | P02-T01, P02-T02, P02-T03, P02-T04, P02-T05 | `tests/queue_lifecycle.rs::queue_lifecycle_moves_incoming_to_processing_to_outgoing`; `tests/orchestrator_workflow_engine.rs::selector_flow_persists_artifacts_and_supports_retry_and_default_fallback`; `tests/provider_runner.rs::mocked_anthropic_success_and_model_mapping` |
| RB-4 | CI gates pass, including E2E and docs checks. | P00-T02, P00-T04, P05-T01, P06-T04 | `tests/release_gate_requirements.rs::release_gate_fails_when_any_blocker_is_violated`; `tests/docs_operator_and_install_readiness.rs::docs_clean_install_smoke_script_runs_in_ci_like_environment` |
| RB-5 | GitHub release workflow publishes expected binaries and checksums. | P05-T02, P05-T03, P05-T04, P06-T04 | `tests/release_gate_requirements.rs::release_gate_passes_with_all_blockers_satisfied`; `tests/release_gate_requirements.rs::release_gate_fails_when_any_blocker_is_violated` |
| RB-6 | User docs and operator docs are complete and validated from clean environment install. | P00-T01, P06-T01, P06-T02, P06-T05 | `tests/docs_operator_and_install_readiness.rs::user_guide_entrypoint_covers_install_first_run_and_integrations`; `tests/docs_operator_and_install_readiness.rs::operator_runbook_includes_service_logs_backup_incident_and_upgrade_procedures`; `tests/docs_operator_and_install_readiness.rs::docs_clean_install_smoke_script_runs_in_ci_like_environment` |
| RB-7 | Placeholder or misleading operational responses are removed. | P04-T03, P06-T04 | `tests/cli_command_surface.rs::daemon_command_surface_works`; `tests/cli_command_surface.rs::failure_modes_unknown_orchestrator_invalid_shared_key_and_invalid_workflow_id` |
