# Slack Setup

This guide walks through connecting Slack to DireClaw with a Slack App using Socket Mode.

## Prerequisites

- `direclaw` is installed and on your `PATH`
- You can run `direclaw setup`
- You have Slack workspace admin access to create/install apps

## 1. Create and configure a Slack App

1. Go to `https://api.slack.com/apps` and create a new app for your workspace.
2. Enable **Socket Mode**.
3. Create an **App-Level Token** with scope `connections:write`.
4. In **OAuth & Permissions**, add bot scopes your deployment needs (typical minimum: `app_mentions:read`, `channels:history`, `chat:write`, `im:history`, `files:read`, `files:write`).
5. Install (or reinstall) the app to the workspace.
6. Copy these values:
- App token (starts with `xapp-`)
- Bot token (starts with `xoxb-`)
- Bot user ID (`U...`) for `slack_app_user_id`

## 2. Export required environment variables

DireClaw Slack runtime expects both tokens in environment variables:

```bash
export SLACK_APP_TOKEN="xapp-..."
export SLACK_BOT_TOKEN="xoxb-..."
```

Persist these in your shell profile or process supervisor so they are present when `direclaw start` runs.

## 3. Bootstrap DireClaw and create an orchestrator

```bash
direclaw setup
direclaw orchestrator add main
```

## 4. Enable Slack channel and add a Slack channel profile

Make sure your `~/.direclaw.yaml` has Slack enabled under `channels`:

```yaml
channels:
  slack:
    enabled: true
```

Then create the Slack channel profile mapped to your orchestrator:

```bash
direclaw channel-profile add slack_main slack main \
  --slack-app-user-id U0123456789 \
  --require-mention-in-channels true
```

Validate:

```bash
direclaw channel-profile show slack_main
```

Expected fields include:

- `channel=slack`
- `orchestrator_id=main`
- `slack_app_user_id=<your bot user id>`
- `require_mention_in_channels=true|false`

## 5. Start and verify runtime state

```bash
direclaw start
direclaw status
```

`status` should show the Slack worker when `channels.slack.enabled=true` and should show channel-profile state.

## 6. Test in Slack

1. Open a DM with the app and send a message.
2. In channels, mention the bot (for example `@YourBot hello`) when `require_mention_in_channels=true`.
3. Check runtime state/logs if needed:

```bash
direclaw logs
```

## Troubleshooting

- `unknown channel profile ...`: create the profile first or use the correct profile id.
- Slack profile validation errors: include both `--slack-app-user-id` and `--require-mention-in-channels`.
- Slack worker not present in `status`: ensure `channels.slack.enabled: true` in `~/.direclaw.yaml`.
- No Slack events: verify app install, scopes, Socket Mode, and that `SLACK_APP_TOKEN` and `SLACK_BOT_TOKEN` are set in the process environment.
