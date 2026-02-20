# Flat Attempt Path Token Optimization Plan

## Summary
This plan reduces prompt/context token overhead by shortening workflow attempt file paths and eliminating redundant filename prefixes. The chosen direction is to store step attempt artifacts directly under the attempt directory (no `provider_prompts/` and no `outputs/` subfolders), with stable short filenames (`prompt.md`, `context.md`, output files by key).

Primary goal:
- Minimize path length in provider instruction messages and context payloads.

Non-goals:
- Backward compatibility for old artifact layouts (project is beta).
- Migration support for prior run directories.

## Run Identifier Format (Base Timestamp + Random Bits)

Adopt compact run identifiers for all new runs:
- Format: `run-<base36_ts>-<rand36_4>`
- `<base36_ts>`: Unix timestamp seconds encoded in lowercase base-36.
- `<rand36_4>`: 4 lowercase base-36 characters from a CSPRNG sample.

Entropy:
- `36^4 = 1,679,616` possibilities.
- Equivalent entropy: `log2(36^4) ~= 20.68` bits.

Example:
- Unix timestamp `1771356898` -> base-36 `tazw56`
- Run id: `run-tazw56-k3f2`

## Current vs Target Layout

### Current
`/Users/dharper/.direclaw/workspaces/main/workflows/runs/<run_id>/steps/<step_id>/attempts/<attempt>/provider_prompts/<run_id>-<step_id>-<attempt>_prompt.md`

`/Users/dharper/.direclaw/workspaces/main/workflows/runs/<run_id>/steps/<step_id>/attempts/<attempt>/provider_prompts/<run_id>-<step_id>-<attempt>_context.md`

`/Users/dharper/.direclaw/workspaces/main/workflows/runs/<run_id>/steps/<step_id>/attempts/<attempt>/outputs/<relative_output_template>`

### Target
`/Users/dharper/.direclaw/workspaces/main/workflows/runs/<run_id>/steps/<step_id>/attempts/<attempt>/prompt.md`

`/Users/dharper/.direclaw/workspaces/main/workflows/runs/<run_id>/steps/<step_id>/attempts/<attempt>/context.md`

`/Users/dharper/.direclaw/workspaces/main/workflows/runs/<run_id>/steps/<step_id>/attempts/<attempt>/<relative_output_template>`

Example (current real run id and compact replacement):

Before:
`/Users/dharper/.direclaw/workspaces/main/workflows/runs/run-sel-msg-1771356898643544000-1771356898/steps/step_1/attempts/1/provider_prompts/run-sel-msg-1771356898643544000-1771356898-step_1-1_context.md`

After:
`/Users/dharper/.direclaw/workspaces/main/workflows/runs/run-tazw56-k3f2/steps/step_1/attempts/1/context.md`

## Design Decisions

1. Prompt/context filenames are constant per attempt:
- `prompt.md`
- `context.md`

2. Output path root moves from attempt `outputs/` folder to attempt root.
- Validation still enforces relative templates and no traversal.
- Resolved paths must remain under the attempt directory.

3. No compatibility fallback for legacy directories.
- Readers and validators only use new layout.

4. Provider instruction messages continue to reference file paths explicitly.
- They now reference shorter paths from the flattened layout.

5. New run identifiers must use base-36 timestamp + random suffix.
- Run id generation in workflow-start transitions is updated from verbose selector-derived id to `run-<base36_ts>-<rand36_4>`.

## Code Changes (Decision-Complete)

## 1) `src/provider/prompt_files.rs`
Update `write_file_backed_prompt`:
- Remove `provider_prompts` directory creation.
- Write files directly under `workspace` argument:
  - `prompt.md`
  - `context.md`
- Return these paths in `PromptArtifacts`.

Behavioral impact:
- Any caller that passes attempt directory as `workspace` gets flat files.
- Selector path usage is also shortened if selector workspace stays unchanged.

## 2) `src/orchestration/output_contract.rs`
Update `resolve_step_output_paths`:
- Change output root from:
  - `.../attempts/<attempt>/outputs`
- To:
  - `.../attempts/<attempt>`

Keep validations unchanged:
- Template must be relative.
- `.` and `..` disallowed.
- Resolved path must start with attempt root.

## 3) `src/orchestration/step_execution.rs`
No interface changes expected, but behavior changes through called helpers:
- `write_file_backed_prompt` now writes to flat attempt directory.
- `output_paths` now resolve directly under attempt directory.

Confirm provider instruction message still points to returned artifact paths.

## 4) `src/orchestration/selector.rs`
Selector prompt/context artifact paths become flatter via the same helper.
No schema changes needed.

## 5) `src/orchestration/prompt_render.rs`
No required schema changes for this plan.
Optional follow-up (not in this change): relative paths in context JSON to reduce tokens further.

## Test Changes

Update path assertions that currently expect `provider_prompts` or `outputs`.

Primary files:
- `tests/orchestrator_workflow_engine.rs`
- `tests/message_flow_queue_orchestrator_provider_e2e.rs`
- `tests/orchestration_prompt_render_module.rs` (if any hardcoded path strings depend on old roots)
- Any tests using `resolve_step_output_paths` expected absolute values.

New/updated assertions:
1. Provider artifacts exist at:
- `.../attempts/1/prompt.md`
- `.../attempts/1/context.md`

2. Output artifacts exist at:
- `.../attempts/1/<template-resolved-file>`
- not under `.../attempts/1/outputs/...`

3. Output path traversal prevention still fails correctly.

## TDD Execution Plan

1. Write failing tests for new prompt/context locations.
2. Write failing tests for flattened output root.
3. Implement helper changes (`prompt_files.rs`, `output_contract.rs`).
4. Update dependent tests.
5. Run full checks in `nix-shell`:
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all`

## Risks and Mitigations

1. Risk: Path collisions in attempt root.
- Mitigation: fixed filenames are safe because each attempt has a unique directory.

2. Risk: Existing tooling/scripts assume old subfolders.
- Mitigation: update test suite and docs together; no compatibility mode.

3. Risk: Output templates may overlap with fixed filenames.
- Mitigation: reserve `prompt.md`, `context.md`, and `provider_invocation.json` as forbidden output template targets (add validation check if not already present).

## Acceptance Criteria

1. No `provider_prompts` directory is created for step attempts.
2. No `outputs` directory is required for step outputs.
3. Prompt/context files are written as `prompt.md` and `context.md` at attempt root.
4. Output path validation still prevents escaping attempt root.
5. All tests, clippy, and formatting pass in `nix-shell`.

## Running log

This is a running log of refactor changes made to iteratively reach the desired structures. Record the date and description of work

2026-02-20 12:34 - Initial document creation
2026-02-20 12:43 - Flattened prompt artifact file layout by updating `write_file_backed_prompt` to write `prompt.md` and `context.md` directly in attempt roots, updated workflow/e2e tests to assert new prompt paths, and stabilized `runtime_supervisor` recovery log assertion with a bounded wait helper so full-suite runs are deterministic.
