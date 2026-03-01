---
name: direclaw-workflow-run-forensics
description: "Use when diagnosing one workflow run's state, step outcomes, retries, and output-contract compliance."
---

# DireClaw Workflow Run Forensics

## Purpose
Explain what happened in a run, where it diverged, and what to do next.

## DireClaw Context
- Run metadata captures state transitions.
- Step artifacts under run work directories provide execution evidence.
- Output contracts are file-based and must be checked by path presence/content.
- Retries and fallback behavior can hide the first real failure if timeline ordering is skipped.

## What To Do
1. Identify run id and target workflow/orchestrator context.
2. Load run metadata and terminal/current state.
3. Inspect each step in execution order, including retries.
4. Validate output contract files per step and final workflow outputs.
5. Build timeline of:
   - step start
   - step completion/failure
   - retry attempts
   - terminal run state
6. Identify first failing/divergent step and causal chain.

## Where To Check
- Run metadata/artifacts:
  - `<current_working_directory>/workflows/runs/<run_id>/`
- Related logs:
  - `<current_working_directory>/logs/`
- Workflow definition for expected contracts:
  - `<current_working_directory>/orchestrator.yaml`

## How To Verify
1. Confirm timeline ordering from timestamps/artifacts.
2. Confirm each claimed failure has direct artifact evidence.
3. Confirm output-file noncompliance with expected contract keys/paths.
4. Re-run smallest reproducible path when possible.

## Boundaries
- Do not generalize from one run to all runs without additional evidence.
- Treat missing artifacts as meaningful findings.
- Do not skip retry history when determining first failure.

## Deliverable
Return:
- Run timeline.
- First divergent/failing step.
- Evidence-backed root cause.
- Minimal next action.
- Confidence level and open unknowns.
