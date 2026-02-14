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

Agents exist to separate responsibilities in a workflow into explicit, reusable execution units.
Instead of one prompt handling everything, each step can call a specific configured agent with a specific role.

Agents are defined per orchestrator in `~/.direclaw/config-orchestrators.yaml` and referenced by workflow steps.

Each agent is responsible for:

- Executing exactly one provider-backed attempt per step invocation (`claude` or `codex`).
- Running with its configured `provider` and `model`.
- Running in the resolved workspace context for that step/run.
- Producing output that the workflow engine can evaluate and route on.

Important boundary:

- Routing decisions stay orchestrator-owned.
- An agent only executes the step it is assigned.
- The `selector_agent` is a normal configured agent with one extra capability flag: `can_orchestrate_workflows: true`.

## Workflows

You can configure custom workflows to achieve any complex task. They're designed to be deterministic in nature and to yield reliable results.

Workflow behavior uses step types:

- `agent_task`: executes a task step using the configured agent.
- `agent_review`: executes a review step and drives branching with `approve` or `reject` outcomes via workflow step routing (`on_approve` / `on_reject`).

Here's an example workflow to show what DireClaw can do:

### A Coding Workflow with Built-In Reviews

```text
+---------------------------------------------------------------+
| User sends feature request to Slack bot                       |
+---------------------------------------------------------------+
                              |
                              v
+---------------------------------------------------------------+
| Orchestrator clones repo in workspace and creates work branch |
+---------------------------------------------------------------+
                              |
                              v
+---------------------------------------------------------------+
| Queue message to Planning Task Agent (`claude` or `codex`)    |
+---------------------------------------------------------------+
                              |
                              v
+---------------------------------------------------------------+
| Planning Agent writes plan                                    |
+---------------------------------------------------------------+
                 |                               ^
                 |                             reject
                 |                               |
                 v                               |
+---------------------------------------------------------------+
| Plan Review Agent reviews plan and returns approve/reject     |
+---------------------------------------------------------------+
                              |
                           approve
                              v
+---------------------------------------------------------------+ 
| Send approved plan + build instructions to Build Agent        | 
+---------------------------------------------------------------+ 
                              |
                              v
+---------------------------------------------------------------+ 
| Build Agent codes furiously                                   | 
+---------------------------------------------------------------+ 
                 |                               ^                
                 |                             reject           
                 |                               |                
                 v                               |                
+---------------------------------------------------------------+ 
| Build Review Agent reviews implementation (approve/reject)    | 
+---------------------------------------------------------------+ 
                              |
                           approve
                              v
+---------------------------------------------------------------+ 
| Push branch and create PR via `gh` CLI                        | 
+---------------------------------------------------------------+ 

```

## Installation

### Prerequisites
- `claude` or `codex` installed and pre-authenticated
- Download the `direclaw` binary from the GitHub Releases page.
- Add the binary to your `PATH` (for example, place it in `/usr/local/bin` on macOS/Linux).

### First-time setup

```bash
direclaw setup
```

This bootstraps runtime state (under `~/.direclaw`) and opens a full-screen setup UI in interactive terminals so you can view and configure:
- workspace path
- primary orchestrator id
- provider/model defaults
- workflow bundle (`minimal`, `engineering`, or `product`)

Setup can be re-run any time to review or update these values. It writes global settings to `~/.direclaw/config.yaml` and orchestrator definitions to `~/.direclaw/config-orchestrators.yaml`.

### Basic lifecycle commands

```bash
direclaw start
direclaw status
direclaw logs
direclaw stop
```

### Common configuration flow
1. In non-interactive setup environments, create an orchestrator:

```bash
direclaw orchestrator add main
```

2. For Slack setup (app creation, required tokens, channel profile wiring), use the user guide:

- [`docs/user-guide/slack-setup.md`](docs/user-guide/slack-setup.md)

3. For headless provider auth artifact sync from 1Password, use:

- [`docs/user-guide/provider-auth-sync-1password.md`](docs/user-guide/provider-auth-sync-1password.md)

4. For production operations (service management, backups, incidents, upgrade/rollback), use:

- [`docs/user-guide/operator-runbook.md`](docs/user-guide/operator-runbook.md)

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
direclaw channels slack sync
direclaw auth sync
direclaw update check
```

Slack runtime uses:

- `SLACK_BOT_TOKEN`
- `SLACK_APP_TOKEN`

Optional profile-specific overrides are also supported:

- `SLACK_BOT_TOKEN_<PROFILE_ID>`
- `SLACK_APP_TOKEN_<PROFILE_ID>`

If multiple Slack profiles are configured, profile-specific token variables are required for each profile.

## Development

### Environment
1. Enter `nix-shell` before any build/test/lint command.
2. Verify toolchain availability:

```bash
rustc --version
cargo --version
```

### Core workflow
1. Read relevant specs in `docs/build/spec/` before implementing.
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
- Index: `docs/build/spec/INDEX.md`
- Runtime/filesystem: `docs/build/spec/01-runtime-filesystem.md`
- Queue processing: `docs/build/spec/02-queue-processing.md`
- Configuration and CLI: `docs/build/spec/09-configuration-cli.md`
- Reliability/testing baseline: `docs/build/spec/12-reliability-compat-testing.md`
