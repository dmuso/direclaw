# Decision Workbook: Open Spec Options

## Purpose

This document captures unresolved option points found in `docs/spec` and provides:

- concrete option framing
- tradeoff analysis for each path
- implementation implications
- explicit fields for final decision and rollout plan

Use this as the working artifact before patching normative spec docs.

## How to Use

For each section:

1. Review the context and option analysis.
2. Fill in `Decision` with the chosen option.
3. Fill in `Way Forward` with exact doc/code follow-up actions.
4. Mark status when complete.

Suggested status values: `open`, `decided`, `implemented`, `verified`.

---

## 1) Workflow Orchestrator Runtime Shape

- Spec reference: `docs/spec/01-runtime-filesystem.md:13`
- Current text: `Workflow orchestrator (standalone process or integrated worker mode)`
- Decision needed: is this a required dual-mode capability, or should one mode be canonical?

### Option A: Standalone process only

- Pros:
  - Stronger isolation and clearer failure boundaries.
  - Simpler operations debugging (single orchestrator service identity).
  - Cleaner scaling path for heavy workflow usage.
- Cons:
  - More process management overhead in small deployments.
  - Slightly more startup/runtime complexity.

### Option B: Integrated worker mode only

- Pros:
  - Simpler deployment topology.
  - Lower runtime footprint for minimal installs.
- Cons:
  - Failure coupling with queue/worker lifecycle.
  - Harder to scale orchestration independently.

### Option C: Support both (with canonical default)

- Pros:
  - Covers simple and advanced deployments.
  - Enables migration path without breaking operators.
- Cons:
  - Broader test matrix and support burden.
  - Must define exact behavior parity requirements.

### Analysis

If operations reliability is a primary release goal, a standalone default is usually safer. If install simplicity is the priority, integrated-only is easier. Supporting both is viable only if mode selection, observability, and failure semantics are explicitly specified.

### Decision

USER: This point has now been incorporated into the spec/task documents and can be considered answered.

---

## 2) Process Supervision Mode

- Spec reference: `docs/spec/01-runtime-filesystem.md:23`
- Current text: `Process supervision may run natively or via tmux-compatibility mode.`
- Decision needed: is tmux compatibility mandatory, optional, or deferred?

### Option A: Native supervisor only

- Pros:
  - Narrower implementation scope.
  - Predictable lifecycle control and logs.
- Cons:
  - Less flexible for users who rely on terminal multiplexing workflows.

### Option B: Native + tmux compatibility required

- Pros:
  - Better operator flexibility.
  - Easier adoption for tmux-heavy environments.
- Cons:
  - Additional compatibility edge cases.
  - More support/testing effort.

### Option C: Native now, tmux as post-GA extension

- Pros:
  - Keeps first release focused.
  - Preserves future direction without blocking now.
- Cons:
  - Requires explicit roadmap and non-goals to avoid ambiguity.

### Analysis

If timeline pressure is high, Option C gives clear boundaries while keeping product direction intact. If tmux workflows are core to target users, Option B may be worth up-front cost.

### Decision

USER: tmux per agent running claude/codex and other tools is mandatory.

---

## 3) Private Workspace Provisioning Behavior

- Spec reference: `docs/spec/01-runtime-filesystem.md:60`
- Current text: `Path must exist or be created during agent provisioning.`
- Decision needed: should runtime auto-create missing private paths outside `agent add`, or require pre-existing paths?

### Option A: Require path exists; fail if missing

- Pros:
  - Safer for explicit infrastructure control.
  - Prevents accidental directory creation in wrong locations.
- Cons:
  - More manual setup friction.
  - Higher risk of startup/config failure in fresh installs.

### Option B: Auto-create at provisioning/setup/start

- Pros:
  - Better user experience and bootstrap reliability.
  - Fewer manual preconditions.
- Cons:
  - Must guard against invalid or unintended paths.
  - Can hide configuration mistakes if not logged clearly.

### Option C: Hybrid (auto-create defaults, require existence for explicit overrides)

- Pros:
  - Convenience for defaults with stricter control for custom paths.
  - Reduces risk of creating arbitrary override paths.
- Cons:
  - More nuanced rules to document and test.

### Analysis

Hybrid rules often balance safety and convenience well, but only if validation/logging are explicit. If security posture is strict, requiring pre-existence for overrides is usually cleaner.

### Decision

USER: Create paths on start if they don't exist.

---

## 4) Workflow `workspace_mode` Default Rule

- Spec reference: `docs/spec/05-workflow-orchestration.md:50`
- Current text: ``workspace_mode` (`run_workspace` default for coding workflows, `agent_workspace`)``
- Decision needed: what is the deterministic default when `workspace_mode` is omitted?

### Option A: Always default to `run_workspace`

- Pros:
  - Deterministic and simple.
  - Strong isolation per workflow run.
- Cons:
  - Some read-only/review steps may lose direct access to agent home context unless explicitly set.

### Option B: Always default to `agent_workspace`

- Pros:
  - Matches direct agent execution environment.
  - Easier access to agent-specific files.
- Cons:
  - Weaker run isolation and reproducibility.
  - Greater risk of cross-run interference.

### Option C: Type-driven default (`agent_task` vs `agent_review`)

- Pros:
  - Can align defaults with intent.
- Cons:
  - Adds hidden logic and complexity.
  - Risks surprising behavior for authors.

### Option D: Require explicit `workspace_mode` for every step

- Pros:
  - No ambiguity.
  - Forces workflow authors to choose intentionally.
- Cons:
  - More verbosity in definitions.

### Analysis

USER: All orchestration agents need a private workspace with configured shared workspaces with other agents. Agents that the orchestration agent manages can use the orchestration space or have an assigned workspace just for that agent as needed. These options should all be reflected in configuration.

---

## 5) Step Timeout Override Semantics

- Spec reference: `docs/spec/05-workflow-orchestration.md:189`
- Current text: `step-level timeout (configured globally and/or overridden)`
- Decision needed: precedence and allowed override ranges.

### Option A: Global timeout only (no per-step override)

- Pros:
  - Minimal complexity.
  - Easy operations predictability.
- Cons:
  - Poor fit for mixed short/long step workloads.

### Option B: Per-step override allowed with global default fallback

- Pros:
  - Practical flexibility.
  - Better fit for heterogeneous workflows.
- Cons:
  - Requires strict validation and precedence rules.

### Option C: Per-step override allowed but clamped by global max

- Pros:
  - Flexibility with safety guardrails.
  - Prevents runaway long steps from config mistakes.
- Cons:
  - More config semantics to explain.

### Analysis

Option C is often the safest production balance: default + override + upper bound. If chosen, the spec should define exact resolution order and units.

### Decision

USER: Option C

---

## 6) Reset Flag Scope and Precedence

- Spec reference: `docs/spec/06-provider-integration.md:17`, `docs/spec/06-provider-integration.md:69`, `docs/spec/06-provider-integration.md:73`
- Current text includes both global and per-agent reset flags.
- Decision needed: how scopes interact if both flags are present.

### Option A: Support only global reset flag

- Pros:
  - Simple mental model.
- Cons:
  - No targeted reset for one agent.

### Option B: Support only per-agent reset flag

- Pros:
  - Fine-grained control.
- Cons:
  - Harder to trigger global maintenance reset.

### Option C: Support both with defined precedence and consumption order

- Pros:
  - Maximum operational control.
- Cons:
  - Must avoid ambiguous double-reset behavior.

### Analysis

If both scopes remain, precedence must be explicit, for example: check per-agent first, then global; consume only the flag that triggered reset; define whether one run can consume both. Without this, behavior will diverge across implementations.

### Decision

USER: I don't care, decide on one and have the orchestrator manage it.

---

## 7) Default Conversation Continuity Policy

- Spec reference: `docs/spec/06-provider-integration.md:36`, `docs/spec/06-provider-integration.md:52`, `docs/spec/06-provider-integration.md:77`
- Current behavior implies resume/continue unless reset.
- Decision needed: should default execution resume prior context or always start fresh?

### Option A: Resume by default; reset flag forces fresh

- Pros:
  - Better continuity for iterative agent usage.
  - Fewer repeated instructions.
- Cons:
  - State drift risk over long sessions.
  - Less deterministic reproductions.

### Option B: Fresh by default; explicit opt-in resume

- Pros:
  - Strong determinism and reproducibility.
  - Lower risk from stale context.
- Cons:
  - Higher token and prompt overhead.
  - Potentially weaker user experience for conversational use.

### Option C: Channel/message mode split (chat resumes, workflows fresh)

- Pros:
  - Tailors behavior to use case.
- Cons:
  - More complex behavior matrix.
  - Must be documented clearly to avoid surprises.

### Analysis

If workflows are expected to be auditable and reproducible, fresh execution for workflow steps is usually preferable. Chat flows may still benefit from resume. A split policy is powerful but must be explicit per execution type.

### Decision

USER: Fresh execution on failed workflow

---

## 8) Slack Mention Requirement Toggle Contract

- Spec reference: `docs/spec/07-channel-adapters.md:59`
- Current text: `target app user is mentioned (unless mention requirement disabled in settings)`
- Decision needed: define the exact setting key, default value, and behavior matrix.

### Option A: Mention required by default; explicit config to disable

- Pros:
  - Safer in shared channels.
  - Reduces accidental triggers.
- Cons:
  - Extra friction for channel-based automation.

### Option B: Mention not required by default

- Pros:
  - Lower friction.
- Cons:
  - Higher noise and accidental execution risk.

### Option C: Mention rule scoped per channel allowlist entry

- Pros:
  - Fine-grained control.
- Cons:
  - More config complexity.

### Analysis

Option A is typically safest for production Slack deployments. Regardless of option, the spec should define precedence among: thread reply, channel allowlist, mention, and mention-toggle behavior.

### Decision

USER: Targeted @ Slack user maps to an orchestration agent who routes/decides from there.

---

## 9) `attach` Command Behavior Outside Supported Supervisors

- Spec reference: `docs/spec/10-daemon-operations.md:54`
- Current text: `Attach to running process supervisor/session where applicable.`
- Decision needed: what should happen when no attachable supervisor/session exists.

### Option A: Hard error with actionable guidance

- Pros:
  - Clear and explicit failure mode.
  - Easier scripting behavior.
- Cons:
  - Less forgiving UX.

### Option B: No-op with informational message

- Pros:
  - User-friendly in mixed environments.
- Cons:
  - Can hide misconfiguration.
  - Worse for automation.

### Option C: Fallback to `logs` tail mode when attach unavailable

- Pros:
  - Still useful operational behavior.
- Cons:
  - `attach` meaning becomes broader/less strict.

### Analysis

For reliable automation and operator clarity, Option A is usually the cleanest unless product intent is explicitly convenience-first. If fallback behavior is chosen, it should be a documented mode, not implicit.

### Decision

USER: Orchestration agent should inspect any workflows/processes and report status back.

---

## Cross-Cutting Follow-Up (after decisions)

- Update normative language in:
  - `docs/spec/01-runtime-filesystem.md`
  - `docs/spec/05-workflow-orchestration.md`
  - `docs/spec/06-provider-integration.md`
  - `docs/spec/07-channel-adapters.md`
  - `docs/spec/10-daemon-operations.md`
- Add any missing config keys and defaults to:
  - `docs/spec/09-configuration-cli.md`
  - `docs/spec/examples/settings/*.json`
- Add acceptance criteria/test coverage updates to:
  - `docs/spec/12-reliability-compat-testing.md`
