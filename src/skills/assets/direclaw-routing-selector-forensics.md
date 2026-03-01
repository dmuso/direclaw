---
name: direclaw-routing-selector-forensics
description: "Use when explaining why a specific inbound message was routed to a workflow, including selector retries and fallback behavior."
---

# DireClaw Routing and Selector Forensics

## Purpose
Reconstruct the selector decision path for one event end-to-end.

## DireClaw Context
- Selector inputs include `availableWorkflows` and `defaultWorkflow`.
- Routing may come from explicit selector choice or fallback (`default_workflow`).
- Evidence lives in selector artifacts, run artifacts, and outbound message records.
- Selector agent must be valid and allowed to orchestrate workflow decisions.

## What To Do
1. Identify target event/message id and time window.
2. Locate selector request/response/retry artifacts for that event.
3. Compare candidate workflows vs selector decision payload.
4. Determine whether outcome was:
   - explicit selector choice
   - fallback to default workflow
   - routing failure/no decision
5. Correlate route decision to created run id and resulting outbound artifact.
6. Explain first divergence point when observed route differs from expected.

## Where To Check
- `<current_working_directory>/orchestrator/select/`
- `<current_working_directory>/workflows/runs/`
- `<current_working_directory>/queue/outgoing/`
- `<current_working_directory>/logs/orchestrator.log`
- Orchestrator config:
  - `<current_working_directory>/orchestrator.yaml`

## How To Verify
1. Confirm selector input contained expected `availableWorkflows`.
2. Confirm returned workflow id is valid and configured.
3. Confirm retry behavior and final outcome match observed routing.
4. Confirm selected/fallback workflow matches run artifact and outbound message.

## Boundaries
- Focus on one event at a time.
- Report uncertainty if artifacts are missing.
- Do not infer selector intent without artifact evidence.

## Deliverable
Return:
- Decision timeline.
- Explicit-selection vs fallback conclusion.
- Evidence paths used for each conclusion.
- First divergence from expected routing (if any).
