---
name: direclaw-log-investigation
description: "Use when reconstructing DireClaw behavior from logs for an incident window, with optional correlation/run/message identifiers."
---

# DireClaw Log Investigation

## Purpose
Build a reliable event timeline and surface likely causes without over-claiming certainty.

## DireClaw Context
Relevant evidence often spans:
- runtime/supervisor logs
- orchestrator logs
- run-specific logs or diagnostic artifacts
- queue state transitions and selector artifacts near the same timestamps

## What To Do
1. Normalize incident window using explicit timezone and absolute timestamps.
2. Gather all relevant logs and artifacts in that window.
3. Filter by run id, message id, workflow id, or correlation id when available.
4. Build a strictly ordered timeline of facts.
5. Mark each causal statement as:
   - observed fact
   - strong inference
   - weak inference
6. Identify first anomaly and likely upstream/downstream impact.

## Where To Check
- `<current_working_directory>/logs/`
- `<current_working_directory>/queue/{incoming,processing,outgoing}/`
- `<current_working_directory>/orchestrator/select/`
- `<current_working_directory>/workflows/runs/`
- Any run-local diagnostics/output files referenced in logs

## How To Verify
1. Confirm timestamps are comparable (same timezone basis).
2. Validate that timeline events map to on-disk artifacts.
3. Re-check at least one alternative hypothesis for excluded causes.
4. Ensure every key conclusion points to a concrete artifact path.

## Boundaries
- Do not treat hypotheses as facts.
- Prefer concise excerpts plus source path references.
- Do not broaden beyond the requested window/scope unless needed to explain root cause.

## Deliverable
Return:
- Timeline summary.
- Key warnings/errors/anomalies.
- Affected runs/messages/workflows.
- Confidence level for each causal claim.
- Open unknowns blocking full certainty.
