# Compatibility Notes (Beta)

As of Phase 18 hardening, DireClaw follows a strict beta compatibility posture:

- No backward-compatibility guarantees for older config/runtime shapes.
- No migration command is provided for pre-hardening layouts.

## Behavior Changes

1. Workflow input config shape is strictly typed.
- Supported: `inputs: [key_a, key_b]`
- Rejected: legacy mapping/object inputs (for example `inputs: { key_a: true }`)

2. Workflow run metadata is persisted only at canonical path.
- Canonical: `~/.direclaw/workflows/runs/<run_id>.json`
- Removed compatibility behavior: mirrored `~/.direclaw/workflows/runs/<run_id>/run.json`

## Remediation

- Update orchestrator configs to the typed `inputs` list shape.
- Use canonical run metadata paths for tooling and operational scripts.
