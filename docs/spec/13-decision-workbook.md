# Decision Workbook: Spec Closure

## Purpose

This document records the final decisions for previously open spec options and links each decision to concrete spec/task updates.

## 1) Workflow Orchestrator Runtime Shape

- Status: `decided`
- Spec reference: `docs/spec/01-runtime-filesystem.md`
- Decision: Support both `standalone` and `integrated` modes with `standalone` as default.
- Rationale: Keeps simple and advanced deployment modes while preserving a canonical operational default.
- Way Forward:
  - Keep dual-mode language in runtime spec.
  - Keep orchestrator as required central dispatch path in all modes.

## 2) Process Supervision Mode

- Status: `decided`
- Spec reference: `docs/spec/01-runtime-filesystem.md`
- Decision: Native supervision and tmux-compatibility are both required; tmux support is mandatory for per-agent provider sessions.
- Rationale: Required by user operating model using tmux for agent/provider execution.
- Way Forward:
  - Update supervisor requirements to `must support` both modes.
  - Add explicit tmux requirement text for per-agent `claude`/`codex` sessions.

## 3) Private Workspace Provisioning Behavior

- Status: `decided`
- Spec reference: `docs/spec/01-runtime-filesystem.md`
- Decision: Create missing private workspace paths at startup/provisioning.
- Rationale: Reduces setup friction and aligns with fail-fast validation for invalid paths while allowing bootstrap on clean installs.
- Way Forward:
  - Keep startup/provisioning directory creation as normative.
  - Keep explicit validation failures for invalid definitions.

## 4) Workflow `workspace_mode` Default Rule

- Status: `decided`
- Spec reference: `docs/spec/05-workflow-orchestration.md`
- Decision: Default `workspace_mode` is `orchestrator_workspace`; supported explicit modes are `orchestrator_workspace`, `run_workspace`, and `agent_workspace`.
- Rationale: Matches requirement that orchestration operates from orchestrator private workspace while still allowing per-step/run alternatives.
- Way Forward:
  - Update workflow field docs and execution workspace rules.
  - Keep workspace access enforcement in config and runtime validation.

## 5) Step Timeout Override Semantics

- Status: `decided`
- Spec reference: `docs/spec/05-workflow-orchestration.md`
- Decision: Option C, per-step overrides allowed and clamped by configured global max.
- Rationale: Allows heterogeneous workflows without permitting unbounded step runtime.
- Way Forward:
  - Add explicit precedence and clamp rules.
  - Require logging when clamps are applied.

## 6) Reset Flag Scope and Precedence

- Status: `decided`
- Spec reference: `docs/spec/06-provider-integration.md`
- Decision: Use per-agent reset flags only; orchestrator manages reset behavior.
- Rationale: Eliminates ambiguous global/per-agent precedence and keeps reset control localized to agent execution context.
- Way Forward:
  - Remove global reset flag behavior.
  - Keep one-shot reset flag consumption semantics.

## 7) Default Conversation Continuity Policy

- Status: `decided`
- Spec reference: `docs/spec/06-provider-integration.md`
- Decision: Enforce fresh execution after failed workflow runs.
- Rationale: Avoids failed-run context contamination while preserving continuity for normal execution.
- Way Forward:
  - Add fresh-on-failure rule before provider invocation.
  - Keep reset-flag forced-fresh behavior.

## 8) Slack Mention Requirement Toggle Contract

- Status: `decided`
- Spec reference: `docs/spec/07-channel-adapters.md`
- Decision: Targeted Slack app-user mentions route to the mapped orchestration agent/profile.
- Rationale: Deterministic profile/orchestrator selection in multi-profile Slack deployments.
- Way Forward:
  - Require mention-to-profile mapping behavior in Slack adapter rules.
  - Keep per-profile credential and outbound reply consistency.

## 9) `attach` Command Behavior Outside Supported Supervisors

- Status: `decided`
- Spec reference: `docs/spec/10-daemon-operations.md`
- Decision: If no attachable session exists, run orchestrator inspection and return workflow/process status summary.
- Rationale: Preserves operational usefulness of `attach` in non-attachable environments.
- Way Forward:
  - Update daemon spec and phase tasks to require status-summary fallback.
  - Validate deterministic behavior in CLI integration tests.
