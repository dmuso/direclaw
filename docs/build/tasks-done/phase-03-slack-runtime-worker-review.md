# Phase 03 Slack Runtime Worker Review

Scope reviewed:
- Uncommitted changes in `src/cli.rs`, `src/runtime.rs`, `src/slack.rs`, `tests/runtime_supervisor.rs`, `tests/slack_channel_sync.rs`
- Requirements in `docs/build/release-readiness-plan.md` and task acceptance in `docs/build/tasks/phase-03-slack-runtime-worker.md`

Validation run:
- `nix-shell --run "cargo test --test slack_channel_sync --test runtime_supervisor"` (pass)

## Findings (Needs Action)

1. Medium: Missing-credential reason is not profile-scoped for single-profile Slack setups.
- Requirement reference: `docs/build/tasks/phase-03-slack-runtime-worker.md:23` requires missing credentials to fail with explicit profile-scoped details.
- Current behavior: when only one Slack profile exists, `load_env_config` emits `MissingEnvVar("SLACK_BOT_TOKEN"|"SLACK_APP_TOKEN")` without profile id context (`src/slack.rs:255`, `src/slack.rs:265`). This propagates to status reason lines (`src/cli.rs:391`) as generic env-var errors.
- Impact: profile-level readiness output can report `auth_missing` but the reason is not explicitly profile-scoped, which does not fully meet the acceptance wording.
- Action:
  - Return profile-qualified credential errors even in single-profile mode (or wrap generic env errors with profile context before surfacing in worker/status paths).
  - Add/adjust an integration assertion that `slack_profile:<id>.reason` includes the profile id for missing credentials.

2. Low: Phase task tracking has not been updated to reflect implemented/tested work.
- Current state: all phase items are still `Status: todo` in `docs/build/tasks/phase-03-slack-runtime-worker.md:16`, `docs/build/tasks/phase-03-slack-runtime-worker.md:29`, `docs/build/tasks/phase-03-slack-runtime-worker.md:42`, `docs/build/tasks/phase-03-slack-runtime-worker.md:55`.
- Impact: release-readiness traceability is incomplete even though implementation/tests are present.
- Action:
  - Update task statuses (`in_progress`/`complete`) and include links to concrete test coverage added in this change set.

## Notes
- No additional functional regressions were identified in the reviewed diff beyond the action items above.
