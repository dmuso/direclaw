# Phase 05 Review: GitHub Release Automation

## Findings

1. **High** - Artifact smoke test does not validate install-from-archive behavior required by Phase 05.
- Spec alignment: `docs/build/tasks/phase-05-github-release-automation.md` (P05-T03 Automated Test Requirements) requires smoke testing install from generated artifacts; `docs/build/release-readiness-plan.md:121` and `docs/build/release-readiness-plan.md:147` require release artifact validation via smoke execution.
- Current implementation: `.github/workflows/release.yml` runs `setup`, and `status` against `target/<triple>/release/direclaw` before packaging, but does not extract and execute from `dist/direclaw-<tag>-<target>.tar.gz`.
- Action required: add an archive-install smoke step that untars the generated artifact and runs `direclaw`, `direclaw setup`, and `direclaw status` from the extracted binary.

2. **Medium** - Checksum generation exists, but checksum verification is not executed in workflow validation.
- Spec alignment: `docs/build/release-readiness-plan.md:147` calls out checksum verification as part of release artifact checks.
- Current implementation: `.github/workflows/release.yml` generates `dist/checksums.txt` but does not run a verification command against it.
- Action required: add a verification step (for example, `shasum -a 256 -c checksums.txt` or equivalent deterministic verification) before publish.

3. **Medium** - Update-command test coverage is missing negative-path metadata parsing/assertion scenarios.
- Spec alignment: `docs/build/tasks/phase-05-github-release-automation.md` (P05-T04 Automated Test Requirements) asks for tests around update metadata parsing and unsupported/apply behavior.
- Current implementation: `tests/update_command.rs` covers successful metadata parsing and up-to-date state only; unsupported `update apply` is asserted in `tests/cli_command_surface.rs`, but no dedicated parsing-failure/draft-release coverage exists.
- Action required: add tests for at least one metadata failure path (invalid JSON or non-200), and draft-release rejection path, to ensure `update check` remains non-misleading under bad release metadata.

## Validation Performed

- Reviewed uncommitted changes in:
  - `.github/workflows/ci.yml`
  - `.github/workflows/release.yml`
  - `.github/release-notes-template.md`
  - `src/cli.rs`
  - `tests/cli_command_surface.rs`
  - `tests/update_command.rs`
- Executed targeted tests in Nix shell:
  - `nix-shell --run 'cargo test --test update_command --test cli_command_surface'` (pass)
