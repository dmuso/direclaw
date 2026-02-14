# Workflow System Full Implementation Plan

## Objective
Deliver a fully working workflow system in DireClaw where orchestrators can run complete multi-step workflows with real step execution, input propagation, output extraction, output-file persistence, transition logic, retries/timeouts, and end-to-end channel integration.

This plan is implementation-focused and test-gated. Completion means workflow execution is operational in production runtime paths, not only represented in config/CLI models.

## Current Gap Summary
Current code includes workflow config schemas, validation, selector routing, and run metadata/progress primitives, but does not fully execute workflow steps end-to-end in runtime.

Critical missing behavior to implement:
- Step execution loop from workflow start through terminal state.
- Runtime persistence and resolution of workflow inputs.
- Prompt rendering with workflow context and inputs.
- Parsing and validation of step outputs from provider responses.
- Output file writes driven by `output_files` mapping.
- Step transitions via `next`, `on_approve`, `on_reject`.
- Retry/timeout/iteration guardrails wired to live execution.
- Full integration through queue runtime and channel adapters.

## Scope and Non-Goals
### In Scope
- Orchestrator runtime workflow engine implementation.
- Step execution for both `agent_task` and `agent_review`.
- Workflow input and output contracts.
- Output file materialization and path safety checks.
- Workflow state/progress persistence.
- Integration with queue processor and provider runners.
- Full automated test coverage including E2E.

### Out of Scope
- New channel adapters beyond Slack for v1.
- Non-workflow feature expansion not required for workflow correctness.

## Target Functional Contract
A workflow run must satisfy all of the following:
1. Inputs supplied at run start are persisted and available to every step.
2. Step prompt receives rendered context including workflow run metadata, inputs, and resolved output file paths.
3. Provider output must contain exactly one `[workflow_result] ... [/workflow_result]` JSON object envelope.
4. Declared outputs are validated and mapped to configured `output_files`.
5. Output files are written under run-scoped output roots and blocked from path traversal/escape.
6. Step transition resolves by:
- `agent_task`: `next` else next sequential step.
- `agent_review`: `on_approve` or `on_reject` based on `decision`.
7. Retry policy, step timeout, run timeout, and max total iteration limits are enforced.
8. Run/progress artifacts are persisted after each attempt and transition.
9. Channel-originated execution uses this same engine path.
10. Terminal states (`succeeded`, `failed`, `canceled`) are deterministic and observable.

## Implementation Architecture
### 1. Run Data Model Completion
Add/extend persisted run structures to store:
- `inputs: Map<String, Value>`.
- `workflow_id`, `current_step_id`, `current_attempt`, total iterations.
- Optional per-step attempt summaries and output indexes.

Files:
- `src/orchestrator.rs` (run store models and persistence)
- `src/config.rs` (ensure schema compatibility)

### 2. Workflow Engine Core
Implement explicit execution engine entrypoints:
- `start_workflow_run(...)`
- `resume_workflow_run(...)`
- `execute_next_step(...)`
- `execute_step_attempt(...)`
- `transition_after_step(...)`

Responsibilities:
- Locate workflow and current step.
- Resolve effective safety limits via orchestrator/workflow/step config.
- Build provider request for the step agent.
- Parse envelope and evaluate transitions.
- Persist attempt records, progress, and state transitions.

Files:
- `src/orchestrator.rs` (new engine functions)
- `src/runtime.rs` (runtime loop integration)

### 3. Prompt Rendering and Context Injection
Create deterministic prompt renderer for step execution:
- Supports `{{inputs.<key>}}` interpolation.
- Supports workflow tokens such as:
  - `{{workflow.run_id}}`
  - `{{workflow.step_id}}`
  - `{{workflow.attempt}}`
  - `{{workflow.run_workspace}}`
  - `{{workflow.output_paths.<key>}}`
  - `{{workflow.output_schema_json}}`
- Produces clear execution context files for provider invocation.

Validation behavior:
- Missing required interpolation keys fail the step attempt with explicit errors.
- Unknown interpolation patterns are surfaced and logged.

Files:
- `src/orchestrator.rs`
- `src/provider.rs` (if context file contract needs extension)

### 4. Output Contract and File Materialization
For each step attempt:
- Parse `[workflow_result]` JSON object.
- Validate against declared `outputs` keys when present.
- Resolve `output_files` paths with existing path safety enforcement.
- Serialize output values and write deterministic files.
- Persist output index in attempt metadata for replayability.

File strategy:
- Text outputs -> UTF-8 text files.
- Object/array outputs -> pretty JSON.
- Scalars -> normalized textual form.

Files:
- `src/orchestrator.rs`
- `src/queue.rs` (only if outbound payload references change)

### 5. Transition Engine
Implement transition resolution rules:
- `agent_task`:
  - if `step.next` exists -> use it.
  - else use lexical next step in workflow definition.
  - else terminal success.
- `agent_review`:
  - parse `decision` from outputs.
  - `approve` -> `on_approve`.
  - `reject` -> `on_reject`.
  - missing target -> terminal failure with explicit config/runtime error.

Guardrails:
- Detect invalid target step IDs.
- Detect loops exceeding max iteration budget.

Files:
- `src/orchestrator.rs`

### 6. Retry, Timeout, and Limits
Wire live enforcement using `resolve_execution_safety_limits` and `enforce_execution_safety`:
- Run timeout enforced before and after attempt execution.
- Step timeout enforced through provider timeout and elapsed checks.
- Retry behavior:
  - Provider failure / parse failure / contract failure increments attempt.
  - Stop when `max_retries` exceeded.
- Max total iterations blocks infinite loops.

Files:
- `src/orchestrator.rs`
- `src/runtime.rs`

### 7. Runtime and Queue Integration
Integrate workflow engine into runtime queue processing:
- On `workflow_start`, create run with inputs and enqueue/continue execution.
- On workflow-bound inbound messages (`workflow_run_id`), resume execution when applicable.
- Keep queue lifecycle atomic (`incoming -> processing -> outgoing`).
- Ensure failures requeue without message loss.

Files:
- `src/runtime.rs`
- `src/orchestrator.rs`
- `src/queue.rs`

### 8. CLI/TUI Parity and Operability
Ensure commands and TUI represent executable behavior, not placeholder config:
- `workflow run` persists inputs and triggers executable flow.
- `workflow status` / `workflow progress` reflect live step-level details.
- `setup` TUI workflow editors already expose required fields; validate runtime uses them.

Files:
- `src/cli.rs`
- `src/cli/setup_tui.rs`

### 9. Observability and Diagnostics
Add explicit operational artifacts and logs:
- Step attempt logs: prompt file path, provider command, exit status, parse errors.
- Transition logs: step -> next step decisions.
- Output file write audit entries.
- Security logs for rejected paths/invalid transitions.

Files:
- `src/runtime.rs`
- `src/orchestrator.rs`

## Delivery Phases and Definition of Done
### Phase 1: Workflow Engine Skeleton
Deliver:
- Engine functions and call graph in place.
- Run input persistence.
- Deterministic step selection and transition scaffolding.

DoD:
- Unit tests compile and pass for run lifecycle primitives.
- No dead code path for core step executor.

### Phase 2: Prompt Rendering + Step Provider Execution
Deliver:
- Prompt/context rendering with input interpolation.
- Provider invocation for step agents.
- Envelope parsing and transition decisioning.

DoD:
- Integration test executes at least one full `agent_task` step.
- Failures produce explicit run failure records.

### Phase 3: Outputs + Output Files
Deliver:
- Output key validation.
- Output-file path resolution and writes.
- Attempt artifacts include output indexes.

DoD:
- Integration tests verify file creation and content correctness.
- Path traversal attempts are blocked and logged.

### Phase 4: Review Loops + Limits/Timeouts
Deliver:
- `agent_review` approval/rejection loops.
- Retry behavior and max retry enforcement.
- Run/step timeout and max-iteration enforcement.

DoD:
- Integration tests cover approve path, reject loop, timeout, and retry exhaustion.

### Phase 5: Runtime Queue + Channel E2E
Deliver:
- Runtime queue processor uses new engine path in production loop.
- Channel-originated workflow execution reaches terminal states with outputs.

DoD:
- E2E tests pass from inbound message to outbound response + output artifacts.
- Restart recovery test confirms safe continuation/requeue semantics.

## Verification Plan
## Test Matrix
### Unit Tests (required)
- Prompt interpolation:
  - `inputs` substitution.
  - workflow token substitution.
  - missing key handling.
- Step result parsing:
  - valid envelope.
  - malformed envelope.
  - non-object JSON rejection.
- Transition resolution:
  - `next` precedence.
  - sequential fallback.
  - review decision mapping.
- Limits:
  - retries, step timeout, run timeout, max iterations.
- Output file path safety:
  - absolute paths blocked.
  - `..` traversal blocked.
  - root escape blocked.

### Integration Tests (required)
- Engine happy paths:
  - single-step task workflow.
  - multi-step workflow with `next` chain.
  - review approve path.
  - review reject loop until approve.
- Failure paths:
  - provider non-zero exit.
  - provider timeout.
  - parse failure.
  - invalid transition target.
  - missing output mapping.
- Persistence checks:
  - run metadata correctness.
  - progress updates per attempt.
  - output file artifact presence and content.

### Queue/Runtime Integration (required)
- Queue lifecycle under workflow execution:
  - incoming claim -> processing -> outgoing.
  - failure requeue with no data loss.
- Restart behavior:
  - stale processing recovery.
  - in-flight workflow run continuity.

### End-to-End Tests (required)
- Channel ingress to workflow completion:
  - inbound channel message triggers selector and workflow start.
  - all steps execute via provider mock runner.
  - outbound response reflects terminal result.
- File-focused E2E:
  - workflow writes multiple outputs to mapped files.
  - files are attached/referenced per outbound semantics.
- Security E2E:
  - malicious `output_files` template rejected with audit log.

### Performance/Soak (required)
- N concurrent workflow runs with independent ordering keys.
- Long reject/approve loop under max-iteration ceiling.
- Measure and assert no deadlocks or queue starvation.

## Coverage and Gate Criteria
All gates must pass in `nix-shell`:
- `cargo fmt --all`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all`

Additional mandatory suites:
- `tests/orchestrator_workflow_engine.rs` expanded to full transition/output coverage.
- `tests/message_flow_queue_orchestrator_provider_e2e.rs` expanded with multi-step + file artifact assertions.
- New E2E fixture suite for review loops and timeout/retry scenarios.

Release block condition for workflow subsystem:
- Any failing test in unit/integration/E2E workflow matrix blocks merge/release.

## Required New/Updated Test Fixtures
Add provider mock fixtures for deterministic responses:
- valid task output envelope
- valid review `approve`
- valid review `reject`
- malformed envelope
- timeout simulation
- non-zero exit simulation

Add workflow config fixtures:
- minimal single-step with outputs
- multi-step task chain
- engineering-style review loop
- malicious output path templates

## Work Breakdown Structure
1. Implement run input persistence and model extensions.
2. Implement step prompt renderer with workflow/input/output-path context.
3. Implement provider-backed step executor with retry logic.
4. Implement output parsing, validation, and output-file writes.
5. Implement transition engine for task/review semantics.
6. Integrate execution path into runtime queue worker.
7. Expand CLI status/progress payloads with step-output context.
8. Add/expand unit tests.
9. Add/expand integration tests.
10. Add/expand E2E and restart recovery tests.
11. Run full gates and fix all regressions.

## Risks and Mitigations
- Risk: Hidden divergence between selector path and step execution path.
- Mitigation: Shared provider invocation utilities and unified logging schema.

- Risk: Prompt/template injection edge cases.
- Mitigation: Strict interpolation parser and exhaustive unit tests.

- Risk: Infinite loops due to review cycles.
- Mitigation: max-iteration hard stop plus explicit failure state.

- Risk: Output file path escape vulnerabilities.
- Mitigation: keep strict relative-path validation and security logging.

## Acceptance Checklist
A workflow system is complete only when all are true:
- Workflows run from first step to terminal state in runtime.
- Inputs are persisted and consumable by step prompts.
- Outputs are parsed, validated, and written to mapped files.
- `agent_review` routes via `on_approve`/`on_reject` correctly.
- Retries/timeouts/iteration limits are enforced.
- Progress and attempt artifacts are accurate and queryable.
- Queue reliability semantics remain intact.
- Full unit + integration + E2E matrix passes.
