# Channel Adapters (Discord/Telegram/WhatsApp/Slack)

## Scope

Defines shared adapter responsibilities and channel-specific behavior for inbound and outbound messaging.

Supported channels:

- `discord`
- `telegram`
- `whatsapp`
- `slack`

## Common Adapter Requirements

All adapters must:

- Accept text and media
- Save inbound media to `~/.direclaw/files`
- Add `[file: /abs/path]` tags to queued message text
- Include `channelProfileId` in queued payload when channel integration uses multiple channel identities
- Track pending requests for response correlation
- Poll outbound queue every second
- Send files before text
- Cleanup pending entries older than 10 minutes
- Support `!agent` and `/agent` commands for configured-agent listing

## Discord Adapter

- Process direct messages only
- Ignore guild messages
- Download attachments from remote attachment URLs
- Split outbound text into chunks of max 2000 chars
- Send typing indicator immediately and refresh every 8s while processing

## Telegram Adapter

- Process private chats only
- Ignore groups/channels
- Support media types:
  - `photo`, `document`, `audio`, `voice`, `video`, `video_note`, `sticker`
- Split outbound text into chunks of max 4096 chars
- Send typing indicator immediately and refresh every 4s while processing

## WhatsApp Adapter

- Ignore group chats
- Support text and supported media message types
- Persist auth session in `~/.direclaw/whatsapp-session`
- Emit QR code to terminal and `~/.direclaw/channels/whatsapp_qr.txt`
- Mark ready state via `~/.direclaw/channels/whatsapp_ready`

## Slack Adapter

- Support Socket Mode
- Support one or more configured Slack channel profiles
- Resolve inbound event `channelProfileId` deterministically from the receiving app profile/credentials
- DMs: always process
- Channels: process only when one condition is true:
  - message is in thread
  - channel is allowlisted
  - target Slack app user is mentioned for the receiving profile
- Mention targeting rule: when multiple Slack profiles are configured, inbound events mentioning a specific app user must map to that profile's `channelProfileId` and therefore to its mapped orchestrator.
- Download files using bearer token auth
- Upload files via Slack file upload API
- Reply in thread context for non-DM messages
- Outbound replies must use the same resolved `channelProfileId` credentials that accepted the inbound event
- Split outbound text around 3500 chars
- For workflow runs, maintain association between `workflowRunId` and Slack thread/conversation id for progress posting
- While associated workflow run is active (`running|waiting`), post progress updates to the same Slack thread every 15 minutes
- Support status-check intent in workflow threads and return latest run progress snapshot.
- Natural-language status intent interpretation must use orchestrator selector-agent inference (same provider CLI path used for workflow selection).
- Support diagnostics intent in workflow threads (for example "why did this fail?" or "investigate what failed") and route to orchestrator `diagnostics_investigate`.
- Exact commands (`status`, `progress`, `/status`, `/progress`) may use adapter/runtime fast-path handling only if behavior is equivalent to selector `workflow_status` action handling.

## Acceptance Criteria

- Inbound events for each channel map to queue payload schema.
- Outbound responses preserve per-channel chunking and threading semantics.
- Adapter commands (`/agent`, `!agent`) return configured agents for the resolved orchestrator/channel profile.
- Workflow dispatch directives never leak directly to end-user channel messages.
