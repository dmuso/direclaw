# Phase 06 Review: Docs Hardening and Release Gates

## Findings

1. **Critical** - Release blocker job cannot run in `publish` workflow because Nix is never installed in that job.
- Spec impact: blocks `docs/build/release-readiness-plan.md` release acceptance criterion **#4** (CI gates pass) and therefore phase-06 release gating.
- Evidence:
  - `.github/workflows/release.yml:124` runs `nix-shell --run 'cargo fmt --all -- --check'`.
  - `.github/workflows/release.yml:132` runs additional `nix-shell` commands.
  - The `publish` job has no `Install Nix` step (contrast with `build` job at `.github/workflows/release.yml:48`).
- Required action:
  - Add an `Install Nix` step in the `publish` job before the first `nix-shell` invocation.
  - Re-run release workflow validation after the fix.

## Open Questions

1. Should phase-06 task status for `P06-T04` remain `complete` before the release workflow is executable end-to-end in CI (`.github/workflows/release.yml`)?
