# Phase 00 Review: Scope and Doc Baseline

Scope reviewed:
- Task implementation in `docs/build/tasks/phase-00-scope-and-doc-baseline.md`
- Alignment against `docs/build/release-readiness-plan.md`
- Related uncommitted docs/tests changes

## Findings Requiring Action

### 1. High: v1 channel scope is still ambiguous in canonical spec docs

- Plan requirement: v1 is Slack-only; Discord/Telegram/WhatsApp are deferred (`docs/build/release-readiness-plan.md`, Summary and Out of Scope for v1).
- Current state: canonical spec still states all four adapters as supported and includes non-Slack adapter milestones.
  - `docs/build/spec/07-channel-adapters.md:1`
  - `docs/build/spec/07-channel-adapters.md:7`
  - `docs/build/spec/12-reliability-compat-testing.md:98`
  - `docs/build/spec/12-reliability-compat-testing.md:99`
- Why this needs action: P00-T01 acceptance says no contradictory statements across README, user guide, and build-spec references. This is not yet true.
- Required action:
  - Mark non-Slack adapters as deferred/post-v1 in spec docs, or
  - Split v1 vs post-v1 requirements explicitly in the spec, and
  - Update docs tests to enforce this boundary.

### 2. Medium: canonical spec path migration is incomplete across internal docs

- Plan/Phase 00 intent: resolve legacy `docs/spec` path mismatch and use canonical `docs/build/spec` references.
- Current state: legacy `docs/build/spec/...` references remain in internal markdown docs.
  - `docs/build/review/review-report-20260214080524.md:5`
  - `docs/build/tasks-done/phase-00-spec-closure.md:13`
  - `docs/build/tasks-done/phase-08-migration-hardening-and-release-readiness.md:35`
- Why this needs action: these are internal repository docs and still perpetuate the non-canonical path.
- Required action:
  - Replace remaining legacy paths with `docs/build/spec/...`, or
  - Clearly label these files archival and exclude them from active-doc validation with a documented rule.

### 3. Medium: traceability index uses non-test placeholders for several release blockers

- Phase 00 requirement: traceability map should include plan requirement -> owning tasks -> tests (file names/test IDs).
- Current state: some `test_references` entries point to phase task markdown notes, not executable test files/IDs.
  - `docs/build/review/requirement-traceability.md:18`
  - `docs/build/review/requirement-traceability.md:19`
  - `docs/build/review/requirement-traceability.md:20`
- Why this needs action: this weakens release-gate auditability; references are not directly runnable/verifiable artifacts.
- Required action:
  - Replace these with concrete test IDs/file targets (or explicit planned workflow checks with stable IDs), and
  - Tighten validator logic in `tests/docs_phase00_baseline.rs` to reject non-test placeholder references.

### 4. Low: docs consistency test coverage does not enforce full P00-T01 acceptance scope

- P00-T01 acceptance includes consistency across README, `docs/user-guide/*`, and build-spec references.
- Current test focus is narrower for unsupported-v1 claims (README + user-guide content scan) and does not enforce build-spec scope consistency for deferred adapters.
  - `tests/docs_phase00_baseline.rs:77`
- Required action:
  - Extend the scope test to include the relevant spec docs (at least `docs/build/spec/07-channel-adapters.md` and `docs/build/spec/12-reliability-compat-testing.md`) with explicit v1-scope assertions.

## Validation Notes

- Executed in Nix shell: `cargo test --test docs_phase00_baseline`
- Result: pass (4/4), indicating structural checks are working but not yet sufficient to catch the semantic gaps above.
