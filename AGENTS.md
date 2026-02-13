# AGENTS

## Project Overview
RustyClaw is a Rust-based, file-backed multi-agent orchestration runtime. It processes channel events through a queue, routes execution through an orchestrator, and runs provider-backed agents with strict workspace and configuration controls.

Primary source of truth for behavior:
- `docs/spec/INDEX.md`
- `docs/spec/01-runtime-filesystem.md` through `docs/spec/12-reliability-compat-testing.md`

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

## Configuration and Runtime Expectations
Implement configuration and runtime behavior to match spec requirements, including:
- State root layout under `~/.rustyclaw`
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

Use `docs/spec/12-reliability-compat-testing.md` as the acceptance baseline.

## Implementation Workflow
1. Read relevant spec section(s) in `docs/spec/` before implementing.
2. Implement the smallest coherent Rust slice.
3. Add or update tests with each change.
4. Run format, lint, and test in `nix-shell`.
5. Keep behavior and naming aligned with spec terminology.
