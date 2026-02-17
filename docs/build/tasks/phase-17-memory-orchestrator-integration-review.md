# Phase 17 Memory Orchestrator Integration Review

Reviewed uncommitted changes against `docs/build/spec/14-memory.md`.

## Findings Requiring Action

1. **High:** Diagnostics memory recall runs even when diagnostics scope is unresolved.
- Spec reference: `docs/build/spec/14-memory.md` ("Diagnostics scope limits and existing safeguards remain enforced.")
- Evidence: `src/orchestration/transitions.rs:335` sets an unresolved-scope response, but memory recall still executes unconditionally at `src/orchestration/transitions.rs:355`.
- Risk: ambiguous diagnostics requests still trigger recall and may include unrelated memory evidence, weakening existing diagnostics scope safeguards.
- Action: gate diagnostics memory recall on a resolved diagnostics scope (or explicitly documented safe fallback scope), and add a regression test for unresolved scope ensuring no recall/evidence retrieval is performed.

2. **Medium:** Workflow output write-back currently supports create-only behavior, not update semantics.
- Spec reference: `docs/build/spec/14-memory.md` ("Parsed workflow outputs can create/update typed memory records.")
- Evidence: memory IDs are attempt-scoped (`src/memory/writeback.rs:88`) and source idempotency is attempt-scoped (`src/memory/writeback.rs:114`), while duplicate source short-circuits node upsert (`src/memory/repository.rs:212`).
- Risk: repeated/changed outputs become new memories per attempt with no update path or supersession linkage, so typed memory updates are not represented.
- Action: introduce stable memory identity/update rules for output-derived memories (for example per run+step+output key semantic identity), and/or explicit `Updates`/status transitions when a newer attempt supersedes earlier output memory.

3. **Medium:** Diagnostics memory evidence artifacts are not persisted on memory recall failure paths.
- Spec reference: `docs/build/spec/14-memory.md` ("Memory evidence artifacts are persisted for replay/audit.")
- Evidence: success path writes `diagnostics-memory-evidence-*.json` (`src/orchestration/transitions.rs:484`), but error branches only log (`src/orchestration/transitions.rs:494`, `src/orchestration/transitions.rs:502`) and do not persist an artifact.
- Risk: replay/audit trail is incomplete for failed diagnostics-memory retrieval attempts.
- Action: persist a deterministic failure artifact (for example with `diagnosticsId`, failure reason, and timestamp) whenever recall/repository access fails.

## Validation Notes

Targeted tests executed in `nix-shell` and passing:
- `cargo test --test orchestration_routing_module`
- `cargo test --test orchestration_prompt_render_module`
- `cargo test --test orchestration_transitions_module`
- `cargo test --test orchestrator_workflow_engine`
