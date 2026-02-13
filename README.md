# DireClaw

DireClaw is an OpenClaw and TinyClaw inspired AI agent platform to get things done.

## How DireClaw differs from OpenClaw

DireClaw is influenced by both OpenClaw and TinyClaw. OpenClaw's complexity introduces many areas where things can break. TinyClaw's approach is to drastically simplify things, and start with a multi-agent approach.

DireClaw draws from both examples to provide deterministic, multi-agent workflows whilst at the same time providing the flexibility and intelligence of AI agents managing the system for you.

## Wrapping Claude and Codex CLI tools

DireClaw is designed to wrap `claude` and `codex` CLI tools for it's AI capabilities. This means that DireClaw currently requires that you have either available on your system and that you have an active subscription and have a logged in OAuth session ready to go.

Being based on these tools means that all of DireClaw's agent communication messaging is file based. This setup is heavily inspired by TinyClaw's approach instead of OpenClaw.

## The Orchestrator

When a new message hits DireClaw, it's handled by an orchestrator. The orchestrator is a specific agent that is designed to perform the following functions:

* Receive messages on a channel and route it to a file-based queue
* Parse user messages and make a decision on internal agent routing paths
* Trigger configured agentic workflows

Every orchestrator has a default workflow that includes one default agent. This will get work done, but the power of DireClaw is to be able to custom define complex agent workflows that are reliable and monitored with deterministic steps.

In Slack, you connect one installed bot to one orchestrator in DireClaw. This allows you to use Slack's `@` command in any DM or public channel to message your orchestrator. Orchestrators will use Slack channel and thread concepts to pull context and reference running workflows.

## Agents

Agents are defined with configuration and are attached to orchestrators and workflows. Agents are of two types: 1. Task and 2. Review. A Task Agent is one that just executes a step and is given input and ouput instructions. A Review Agent is one that does a task with a defined set of ouptut states: `approve` | `reject`. These outputs are used in workflows to decide on next steps.

## Workflows

You can configure custom workflows to achieve any complex task. They're designed to be deterministic in nature and to yield reliable results.

Here's an example workflow to show what DireClaw can do:

### A Coding Workflow with Built-In Reviews

1. A user messages a Slack bot requesting a new feature build for a codebase.
2. The codebase is cloned to a local folder within the orchestrator's workspace and a branch is created for the work with a name that reflects the request.
3. A message is sent via the file queue to a Task Agent to plan the feature via `claude` or `codex`.
4. The Task Agent starts the CLI tool and provides path references to the input message, clear instructions and prompts for the work requested, and instructions on generating an output message file via a specific path.
5. This output message file is send to a Review Agent that reviews the work done via `claude` or `codex` and outputs a report to a specific file path with an `approve` or `reject` decision.
6. If the plan is approved, we move to the next step, if rejected, the feedback is sent back to the "planning agent" for rework and re-review.
7. Once a plan is approved, the path to the plan document, and clear instructions to build are sent to the build agent, with a specific path to output a summary of work completed.
8. The path to the output summary file and instructions to review are given to a review agent.
9. The review agent reviews in the same manner as the plan review, except the instructions are specific to reviewing code that has been generated.
10. If the review agent approves, we move to the next step, otherwise the review feedback goes back to the build agent for rework and re-review.
11. Once the work is considered done, the branch is pushed and a pull request is created via the `gh` CLI tool.

## Installation

### Prerequisites
- `claude` or `codex` installed and pre-authenticated
- Download the `direclaw` binary from the GitHub Releases page.
- Add the binary to your `PATH` (for example, place it in `/usr/local/bin` on macOS/Linux).

### First-time setup

```bash
direclaw setup
```

This bootstraps runtime state (under `~/.direclaw`) and creates a default global config at `~/.direclaw.yaml` if it does not already exist.

### Basic lifecycle commands

```bash
direclaw start
direclaw status
direclaw logs
direclaw stop
```

### Common configuration flow
1. Create an orchestrator:

```bash
direclaw orchestrator add main
```

2. For Slack setup (app creation, required tokens, channel profile wiring), use the user guide:

- [`docs/user-guide/slack-setup.md`](docs/user-guide/slack-setup.md)

### Useful commands

```bash
direclaw orchestrator list
direclaw orchestrator show <orchestrator_id>
direclaw orchestrator-agent list <orchestrator_id>
direclaw workflow list <orchestrator_id>
direclaw workflow run <orchestrator_id> <workflow_id> --input key=value
direclaw workflow status <run_id>
direclaw workflow progress <run_id>
direclaw channels reset
direclaw update check
```

## Development

### Environment
1. Enter `nix-shell` before any build/test/lint command.
2. Verify toolchain availability:

```bash
rustc --version
cargo --version
```

### Core workflow
1. Read relevant specs in `docs/spec/` before implementing.
2. Make the smallest coherent Rust change.
3. Add or update tests for the behavior.
4. Run quality checks from inside `nix-shell`:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

### Design constraints to preserve
- Route all channel-originated execution through the orchestrator path.
- Keep queue lifecycle semantics (`incoming -> processing -> outgoing`) correct and atomic where possible.
- Enforce workspace isolation and shared-workspace allowlists from config.
- Fail fast on invalid config/workspace/orchestrator definitions.
- Keep CLI behavior aligned with the spec-defined command surface.

### Spec references
- Index: `docs/spec/INDEX.md`
- Runtime/filesystem: `docs/spec/01-runtime-filesystem.md`
- Queue processing: `docs/spec/02-queue-processing.md`
- Configuration and CLI: `docs/spec/09-configuration-cli.md`
- Reliability/testing baseline: `docs/spec/12-reliability-compat-testing.md`
