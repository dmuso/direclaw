# Release Checklist (v1 Go/No-Go)

Canonical requirement source:
- `docs/build/release-readiness-plan.md`

Complete every item before tagging `v1.0.0`.

## RB-1 Runtime Workers Under Daemon Lifecycle

- [ ] `cargo test --all` passes runtime lifecycle suites.
- [ ] `direclaw start|status|stop|restart` verified on clean HOME.
- [ ] Runtime state files/logs reflect actual worker health.

## RB-2 Slack End-to-End Flow Verified

- [ ] Slack sync integration tests pass.
- [ ] Inbound-to-outbound workflow path validated in automated tests.
- [ ] Channel-profile mapping and mention rules verified.

## RB-3 Queue/Orchestrator/Provider Production Path

- [ ] Queue lifecycle tests pass for `incoming -> processing -> outgoing`.
- [ ] Orchestrator/provider tests verify selector + workflow execution.
- [ ] Restart/recovery coverage confirms no stranded processing files.

## RB-4 CI Gates (Tests + Docs)

- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --all` passes.
- [ ] Docs validation suites pass (links, task structure, phase-06 docs checks).

## RB-5 Release Artifacts and Checksums

- [ ] Build matrix artifacts generated for all release targets.
- [ ] `checksums.txt` generated and verified.
- [ ] Archive smoke test (`direclaw`, `setup`, `status`) passes per target.

## RB-6 User and Operator Docs from Clean Install

- [ ] User guide includes binary install, first-run, Slack setup, auth sync, troubleshooting.
- [ ] Operator runbook includes service management, logs/backups, incidents, upgrade/rollback.
- [ ] Clean-environment docs smoke script passes.

## RB-7 No Placeholder or Misleading Operational Responses

- [ ] Unsupported flows fail explicitly with remediation (for example `update apply`).
- [ ] Release traceability contains concrete automated test references (no `planned:` placeholders).
- [ ] Release notes/templates used for publish contain no unresolved placeholder tokens.
