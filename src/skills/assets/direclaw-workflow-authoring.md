---
name: direclaw-workflow-authoring
description: "Use when creating or modifying workflow definitions while preserving DireClaw graph and output-contract invariants."
---

# DireClaw Workflow Authoring

## Purpose
Produce safe workflow graph changes with predictable runtime behavior.

## DireClaw Context
- Workflow steps each use one configured agent.
- Transition keys (`next`, `on_approve`, `on_reject`) define graph flow.
- Outputs are file-based contracts consumed downstream.
- Invalid step references or transition paths can break runs at dispatch or mid-execution.

## What To Do
1. Identify target workflow(s) and requested behavior change.
2. Edit only affected workflow keys/steps/transitions.
3. Validate identifiers:
   - workflow ids
   - step ids
   - agent ids
4. Validate graph integrity:
   - reachable start path
   - no unintended dead ends
   - intentional loops only
5. Validate output contracts:
   - `outputs` definitions
   - `output_files` mappings
   - deterministic file path instructions in prompts
6. Re-check selector/default workflow references if routing behavior changed.

## Where To Change
- Workflow definitions in:
  - `<current_working_directory>/orchestrator.yaml`
  - workflow template files referenced by orchestrator config (if present)

## How To Verify
1. Static validation of workflow and agent references.
2. Run a minimal workflow execution path for changed step(s).
3. Confirm expected transitions occurred.
4. Confirm expected output files exist with parseable content.

## Boundaries
- Keep edits scoped to requested workflow behavior.
- Do not mix unrelated config rewrites into workflow work.
- Do not introduce implicit output contracts via stdout.

## Deliverable
Return:
- Workflow diff summary.
- Invariant check results.
- Expected execution impact.
- Any required follow-up for prompt/contract alignment.
