# Operator Runbook

This runbook covers production operation of DireClaw services without requiring codebase knowledge.

## Scope

- Service management with `systemd` (Linux) and `launchd` (macOS)
- Logging and backup operations
- Incident response procedures
- Upgrade and rollback for post-v1 releases

## Service Management

### Linux (`systemd`)

Example unit file at `/etc/systemd/system/direclaw.service`:

```ini
[Unit]
Description=DireClaw Runtime
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=direclaw
Environment=HOME=/home/direclaw
Environment=SLACK_APP_TOKEN=xapp-...
Environment=SLACK_BOT_TOKEN=xoxb-...
ExecStart=/usr/local/bin/direclaw start
ExecStop=/usr/local/bin/direclaw stop
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Commands:

```bash
sudo systemctl daemon-reload
sudo systemctl enable direclaw
sudo systemctl start direclaw
sudo systemctl status direclaw
sudo journalctl -u direclaw -f
```

### macOS (`launchd`)

Example plist at `/Library/LaunchDaemons/com.direclaw.runtime.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.direclaw.runtime</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/direclaw</string>
    <string>start</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HOME</key>
    <string>/Users/direclaw</string>
    <key>SLACK_APP_TOKEN</key>
    <string>xapp-...</string>
    <key>SLACK_BOT_TOKEN</key>
    <string>xoxb-...</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>/Users/direclaw/.direclaw/logs/launchd.stdout.log</string>
  <key>StandardErrorPath</key>
  <string>/Users/direclaw/.direclaw/logs/launchd.stderr.log</string>
</dict>
</plist>
```

Commands:

```bash
sudo launchctl bootstrap system /Library/LaunchDaemons/com.direclaw.runtime.plist
sudo launchctl enable system/com.direclaw.runtime
sudo launchctl kickstart -k system/com.direclaw.runtime
sudo launchctl print system/com.direclaw.runtime
```

## Logs and State

- State root: `~/.direclaw`
- Main logs: `~/.direclaw/logs`
- Security events: `~/.direclaw/logs/security.log`
- Queue directories: `~/.direclaw/queue/incoming`, `~/.direclaw/queue/processing`, `~/.direclaw/queue/outgoing`

Useful checks:

```bash
direclaw status
direclaw logs
find ~/.direclaw/queue -maxdepth 2 -type f | sort
```

## Backup Strategy

Minimum backup set:

- `~/.direclaw/config.yaml`
- `~/.direclaw/config-orchestrators.yaml`
- `~/.direclaw/channel_profiles/*.yaml`
- `~/.direclaw/shared_workspaces.yaml`

Suggested backup command:

```bash
STAMP="$(date +%Y%m%d-%H%M%S)"
tar -czf "direclaw-backup-${STAMP}.tar.gz" \
  ~/.direclaw/config.yaml \
  ~/.direclaw/orchestrators \
  ~/.direclaw/channel_profiles \
  ~/.direclaw/shared_workspaces.yaml
```

## Incident Procedures

### Startup failure

1. Run `direclaw doctor`.
2. Validate config path and syntax (`~/.direclaw/config.yaml`).
3. Confirm required environment variables are present in supervisor context.
4. Run `direclaw status` and `direclaw logs`.

### Slack ingestion stall

1. Run `direclaw channels slack sync` manually.
2. Confirm Slack app tokens and app install/scopes.
3. Check queue pressure:

```bash
find ~/.direclaw/queue/incoming -maxdepth 1 -type f | wc -l
find ~/.direclaw/queue/processing -maxdepth 1 -type f | wc -l
find ~/.direclaw/queue/outgoing -maxdepth 1 -type f | wc -l
```

4. Restart runtime after fixing root cause:

```bash
direclaw restart
```

### Workspace access denied errors

1. Validate orchestrator `private_workspace` path and shared workspace grants.
2. Confirm shared workspace key exists in `shared_workspaces`.
3. Re-apply grants:

```bash
direclaw orchestrator grant-shared-access <orchestrator_id> <shared_key>
direclaw orchestrator show <orchestrator_id>
```

## Upgrade and Rollback

### Upgrade (post-v1 releases)

1. Download target release archive and `checksums.txt`.
2. Verify checksums:

```bash
shasum -a 256 -c checksums.txt
```

3. Stop service:

```bash
direclaw stop
```

4. Replace binary atomically:

```bash
install -m 0755 direclaw /usr/local/bin/direclaw
```

5. Start and validate:

```bash
direclaw start
direclaw status
direclaw update check
```

### Rollback

1. Stop service.
2. Reinstall previous known-good binary and verify checksum.
3. Restore latest configuration backup if needed.
4. Start runtime and validate:

```bash
direclaw start
direclaw status
```

5. Record rollback reason and attach `status` + relevant log excerpts in incident tracking.
