# User Guide

This guide is the primary entrypoint for first-time DireClaw operators using release binaries.

## v1 Scope

DireClaw v1 supports Slack only.

## Deferred After v1

- Discord
- Telegram
- WhatsApp

## Navigation

- [Install and First Run](#install-and-first-run)
- [Installation and Setup (Docker on Debian Bookworm Slim)](installation-and-setup.md)
- [Slack Setup](slack-setup.md)
- [Provider Auth Sync (1Password)](provider-auth-sync-1password.md)
- [Operator Runbook](operator-runbook.md)
- [Troubleshooting Matrix](#troubleshooting-matrix)

## Install and First Run

Follow this in order from a clean host.

### 1. Download and install the release binary

1. Open GitHub Releases and download the archive for your platform:
- macOS arm64: `direclaw-<tag>-aarch64-apple-darwin.tar.gz`
- Linux x86_64: `direclaw-<tag>-x86_64-unknown-linux-musl.tar.gz`
- Linux arm64: `direclaw-<tag>-aarch64-unknown-linux-musl.tar.gz`
2. Download `checksums.txt` from the same release.
3. Verify integrity:

```bash
shasum -a 256 -c checksums.txt
```

4. Extract and install:

```bash
tar -xzf direclaw-<tag>-<target>.tar.gz
chmod +x direclaw
sudo mv direclaw /usr/local/bin/direclaw
```

5. Confirm install:

```bash
direclaw
```

### 2. Bootstrap runtime state

```bash
direclaw setup
```

This initializes runtime state under `~/.direclaw` and writes `~/.direclaw/config.yaml` when absent.
In interactive terminals, `setup` opens a full-screen menu to view/edit workspace, orchestrators, provider/model defaults, and workflow template choices.

### 3. Verify your orchestrator

```bash
direclaw orchestrator list
```

`direclaw setup` creates the default `main` orchestrator on clean installs.

### 4. Configure Slack profile and auth

1. Complete Slack app setup and collect required values using [Slack Setup](slack-setup.md).
2. Set required Slack environment variables before starting runtime:

```bash
export SLACK_APP_TOKEN="xapp-..."
export SLACK_BOT_TOKEN="xoxb-..."
```

3. Create a Slack channel profile bound to your orchestrator:

```bash
direclaw channel-profile add slack_main slack main \
  --slack-app-user-id U0123456789 \
  --require-mention-in-channels true
```

### 5. Optional provider auth sync for headless hosts

If provider login material is stored in 1Password, configure [Provider Auth Sync (1Password)](provider-auth-sync-1password.md) and run:

```bash
direclaw auth sync
```

### 6. Start runtime and validate first message flow

```bash
direclaw start
direclaw status
direclaw channels slack sync
```

Then send a DM to the Slack app (or mention it in an allowed channel) and verify runtime health:

```bash
direclaw logs
```

For production service supervision, continue with [Operator Runbook](operator-runbook.md).

## Troubleshooting Matrix

| Symptom | Likely Cause | Remediation |
|---|---|---|
| `direclaw: command not found` | Binary not installed on `PATH` | Install to `/usr/local/bin` (or another `PATH` directory) and reopen shell. |
| `unknown channel profile` | Profile id typo or profile missing | Run `direclaw channel-profile list` and recreate profile with `direclaw channel-profile add ...`. |
| Slack worker missing in `status` | Slack channel disabled in config | Set `channels.slack.enabled: true` in `~/.direclaw/config.yaml`, then `direclaw restart`. |
| `SLACK_*_TOKEN... required` errors | Required Slack token env vars not present in runtime process | Export `SLACK_APP_TOKEN` and `SLACK_BOT_TOKEN` (plus profile-scoped overrides for multi-profile setups). |
| `missing_scope` for `im:read` during sync | Slack app token lacks DM-read scope while runtime still polls DM conversations | Add `channels.slack.include_im_conversations: false` to `~/.direclaw/config.yaml` (disables DM polling), or grant `im:read` and reinstall app. |
| `auth sync failed` | `op` CLI missing, token missing, or secret reference invalid | Install `op`, export `OP_SERVICE_ACCOUNT_TOKEN`, and validate each `auth_sync.sources.*.reference`. |
| No outbound Slack replies | Slack app scopes/mode incomplete or app not reinstalled | Re-check Socket Mode, OAuth scopes, reinstall app, run `direclaw channels slack sync`. |
| `update apply is unsupported...` | In-place self-update intentionally blocked | Download new release archive manually, verify checksum, replace binary. |
