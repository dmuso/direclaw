# Provider Auth Sync (1Password)

This guide configures DireClaw to pull provider auth artifacts (for example CLI session files) from 1Password at runtime.

## Why use this

In headless deployments, interactive OAuth on the host is hard to operate.  
`auth_sync` lets DireClaw fetch pre-generated auth files from 1Password using a service account token.

## Prerequisites

- `direclaw` installed
- `op` (1Password CLI) installed and available in `PATH`
- 1Password service account token provided via environment variable:

```bash
export OP_SERVICE_ACCOUNT_TOKEN="ops_..."
```

- A 1Password secret reference for each auth artifact (for example `op://DireClaw/codex-auth/document`)

## 1. Configure `auth_sync` in `~/.direclaw/config.yaml`

Example:

```yaml
auth_sync:
  enabled: true
  sources:
    codex:
      backend: onepassword
      reference: op://DireClaw/codex-auth/document
      destination: "~/.codex/auth.json"
      owner_only: true
    claude:
      backend: onepassword
      reference: op://DireClaw/claude-auth/document
      destination: "~/.config/claude/auth.json"
      owner_only: true
```

## 2. Run a manual sync

```bash
direclaw auth sync
```

Expected output:

- `auth sync complete`
- `sources=<count>`
- `sources=<source ids>`

## 3. Start runtime with automatic sync

```bash
direclaw start
```

`start` runs auth sync before workers are marked running.  
Startup output includes `auth_sync=...` status.

## Validation rules

When `auth_sync.enabled=true`:

- `sources` must be non-empty
- `backend` currently must be `onepassword`
- `reference` must be non-empty
- `destination` must be absolute or start with `~/`

## Security behavior

- Source content is fetched via `op read <reference>`.
- Files are written via temp file + atomic rename.
- On Unix, `owner_only: true` applies `0600` permissions.
- DireClaw does not log secret content.

## Troubleshooting

- `OP_SERVICE_ACCOUNT_TOKEN is required for auth sync`: set/export token in the runtime environment.
- ``auth sync failed: `op` binary is not available in PATH``: install `op` and ensure it is on `PATH`.
- `auth sync source ... failed to read ...`: verify 1Password reference path and service account permissions.
- `destination ... must be absolute or start with ~/`: fix config path format.
