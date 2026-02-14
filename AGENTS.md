# AGENTS

## Project Overview
DireClaw is a Rust-based, file-backed multi-agent orchestration runtime. It processes channel events through a queue, routes execution through an orchestrator, and runs provider-backed agents with strict workspace and configuration controls.

Primary source of truth for behavior:
- `docs/build/spec/INDEX.md`
- `docs/build/spec/01-runtime-filesystem.md` through `docs/build/spec/12-reliability-compat-testing.md`

## Project Status
This project is in beta stage. We do not support backwards compatibility or migration support for older config files or contracts. Therefore any code for backwards compatibility or migration is not needed.

## Tech Stack
- Language: Rust
- Environment: Nix shell (`shell.nix`)
- Core tooling: `cargo`, `rustfmt`, `clippy`

## Required Dev Environment
Use Nix shell before running any build/test/lint command.

```bash
nix-shell
```

Inside the shell, verify toolchain availability:

```bash
rustc --version
cargo --version
```

## Engineering Constraints
- Route all channel-originated execution through the orchestrator path.
- Preserve queue lifecycle semantics (`incoming -> processing -> outgoing`) with atomic moves where possible.
- Enforce workspace isolation and shared-workspace allowlists from config.
- Fail fast on invalid config paths and orchestrator/workspace definitions.
- Keep CLI behavior aligned with spec-defined command surface.

## The Domain
- A Slack bot is 1:1 with an Orchestrator
- An Orchestrator has many Workflows
- An Orchestrator has many Agents
- Workflow steps have one Agent assigned
- Agents have one configured Provider and Model
- Workflows can be complex, so choosing from a Workflow Templates is a nice thing

## Agents

- Agents are wrappers over the execution of `claude` or `codex` CLI tools.
- These tools run in an agentic loop and can read/write multiple files in one process.
- Due to this behaviour, you cannot expect a simple JSON in/JSON out pattern.
- Instead, you must instruct agents to read and write from files for input/output.
- Prompt context can also be given in an input, but output is strictly file only.
- When prompting, an agent must be told the exact path to write a unique output file that contains the desired output

### Agent Output Prompt Examples

#### Invalid

> "Return exactly one [workflow_result] JSON envelope."
    
This is invalid because it refers to a "return" concept that then infers stdout output. `claude` and `codex` will output to stdout, but it will contain all of it's thinking and processing, not the desired structured output.

#### Valid

> "Once the request is completed, write a JSON file to <path> containing a summary of the change. Include [other instructions]..."

This prompt instructs a specific, deterministic file output that the workflow can read to make a decision on the next step.

## Configuration and Runtime Expectations
Implement configuration and runtime behavior to match spec requirements, including:
- State root layout under `~/.direclaw`
- Global `settings.yaml` + per-orchestrator `orchestrator.yaml`
- Orchestrator private workspace resolution
- Shared workspace registry and per-orchestrator grants
- Long-lived workers (channels, queue processor, orchestrator, optional heartbeat)

## Code Quality Rules
- Format: `cargo fmt --all`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- Tests: `cargo test --all`

Run all checks from `nix-shell`.

## Testing Priorities
Prioritize automated coverage for:
- Queue lifecycle and recovery behavior
- Selector/orchestrator routing and workflow dispatch
- Workspace access enforcement
- CLI command parity and config validation
- Adapter mappings and file round-trip behavior

Use `docs/build/spec/12-reliability-compat-testing.md` as the acceptance baseline.

## Implementation Workflow
1. Read relevant spec section(s) in `docs/build/spec/` before implementing.
2. Implement the smallest coherent Rust slice.
3. Add or update tests with each change.
4. Run format, lint, and test in `nix-shell`.
5. Keep behavior and naming aligned with spec terminology.
