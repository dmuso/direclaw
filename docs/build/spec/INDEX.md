# DireClaw Core Specs Index

This index defines the canonical feature-spec set for DireClaw.
Source baseline: `docs/SPEC.md`.

## How To Use This Spec Set

- This index is the entry point for product, engineering, and QA.
- Each linked document is a normative feature specification.
- If requirements conflict, this precedence order applies:
  1. `docs/build/spec/INDEX.md` (this file)
  2. Feature spec files in this folder
  3. `docs/SPEC.md` (legacy consolidated baseline)

## Feature Specs

1. Runtime and Filesystem Model
   `docs/build/spec/01-runtime-filesystem.md`
2. Queue Processing and Message Lifecycle
   `docs/build/spec/02-queue-processing.md`
3. Agent Routing and Execution
   `docs/build/spec/03-agent-routing-execution.md`
4. Workspace Access and Isolation
   `docs/build/spec/04-workspace-access.md`
5. Workflow Orchestration
   `docs/build/spec/05-workflow-orchestration.md`
6. Provider Integration (Anthropic/OpenAI)
   `docs/build/spec/06-provider-integration.md`
7. Channel Adapters (v1: Slack; post-v1: Discord/Telegram/WhatsApp)
   `docs/build/spec/07-channel-adapters.md`
8. File Exchange and Attachment Semantics
   `docs/build/spec/08-file-exchange.md`
9. Configuration and Management Commands
   `docs/build/spec/09-configuration-cli.md`
10. Daemon Lifecycle and Operations
    `docs/build/spec/10-daemon-operations.md`
11. Heartbeat Automation Service
    `docs/build/spec/11-heartbeat-service.md`
12. Reliability, Compatibility, and Testing
    `docs/build/spec/12-reliability-compat-testing.md`

## Working Decision Docs

- `docs/build/spec/13-decision-workbook.md`

## Example Prompt Templates

- `docs/build/spec/examples/prompts/workflow_selector_minimal.prompt.md`
- `docs/build/spec/examples/prompts/workflow_selector_rich.prompt.md`

## Example Settings Configs

- `docs/build/spec/examples/settings/minimal.settings.yaml`
- `docs/build/spec/examples/settings/full.settings.yaml`

## Example Orchestrator Configs

- `docs/build/spec/examples/orchestrators/minimal.orchestrator.yaml`
- `docs/build/spec/examples/orchestrators/engineering.orchestrator.yaml`
- `docs/build/spec/examples/orchestrators/product.orchestrator.yaml`

## Product Coverage

These feature specs collectively cover:

- Channel adapters: `slack` in v1, with `discord`/`telegram`/`whatsapp` deferred post-v1
- File-backed queue processing
- Multi-agent routing and execution
- Orchestrator-managed workflows
- Hybrid workspaces (private + shared)
- Provider execution through local CLIs (`claude`, `codex`)
- Heartbeat automation
- Daemonized runtime operations
- Compatibility posture and reliability requirements
- Test strategy and release milestones
