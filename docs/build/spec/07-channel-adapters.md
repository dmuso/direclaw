# Channel Adapters

## Scope

Defines adapter responsibilities and channel-specific behavior for inbound and outbound messaging.

DireClaw v1 supports Slack only.
Discord, Telegram, and WhatsApp are deferred after v1 and remain documented here as post-v1 targets.

## v1 Supported Channel

- `slack`

## Common Adapter Requirements

When implemented, each adapter must:

- Accept text and media
- Save inbound media to `<orchestrator_runtime_root>/files` for the resolved orchestrator
- Add `[file: /abs/path]` tags to queued message text
- Include `channelProfileId` in queued payload when channel integration uses multiple channel identities
- Track pending requests for response correlation
- Poll outbound queue every second
- Send files before text
- Cleanup pending entries older than 10 minutes
- Support `!agent` and `/agent` commands for configured-agent listing

`<orchestrator_runtime_root>` resolves to the orchestrator private workspace root for the message's resolved channel profile.

## Slack Adapter

- Support Socket Mode
- Socket Mode is the primary inbound path for runtime workers.
- Outbound delivery remains Slack Web API (`chat.postMessage`).
- `conversations.history` polling is optional backfill only (`poll` or `hybrid` modes).
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
- Unified targeted outbound contract for Slack-bound actions must use:
  - `targetRef.channel = "slack"`
  - `targetRef.channelProfileId` (required)
  - `targetRef.channelId` (required)
  - `targetRef.threadTs` (required when `postingMode=thread_reply`)
  - `targetRef.postingMode` (`channel_post|thread_reply`)
- Adapter delivery must use one canonical targeted-post function for both channel posts and thread replies.
- Split outbound text around 3500 chars
- Required Slack app configuration:
  - Socket Mode enabled
  - App-level token (`xapp-...`) with connections permission
  - Bot token (`xoxb-...`) with channel/DM history and write scopes for supported conversation types
- For workflow runs, maintain association between `workflowRunId` and Slack thread/conversation id for progress posting
- While associated workflow run is active (`running|waiting`), post progress updates to the same Slack thread every 15 minutes
- Support status-check intent in workflow threads and return latest run progress snapshot.
- Natural-language status intent interpretation must use orchestrator selector-agent inference (same provider CLI path used for workflow selection).
- Support diagnostics intent in workflow threads (for example "why did this fail?" or "investigate what failed") and route to orchestrator `diagnostics_investigate`.
- Exact commands (`status`, `progress`, `/status`, `/progress`) may use adapter/runtime fast-path handling only if behavior is equivalent to selector `workflow_status` action handling.

## Deferred After v1 (Post-v1 Targets)

### Discord Adapter

- Process direct messages only
- Ignore guild messages
- Download attachments from remote attachment URLs
- Split outbound text into chunks of max 2000 chars
- Send typing indicator immediately and refresh every 8s while processing

### Telegram Adapter

- Process private chats only
- Ignore groups/channels
- Support media types:
  - `photo`, `document`, `audio`, `voice`, `video`, `video_note`, `sticker`
- Split outbound text into chunks of max 4096 chars
- Send typing indicator immediately and refresh every 4s while processing

### WhatsApp Adapter

- Ignore group chats
- Support text and supported media message types
- Persist auth session in `~/.direclaw/whatsapp-session`
- Emit QR code to terminal and `~/.direclaw/channels/whatsapp_qr.txt`
- Mark ready state via `~/.direclaw/channels/whatsapp_ready`

## Acceptance Criteria

- v1 inbound/outbound behavior is fully supported for Slack.
- Non-Slack adapter requirements are explicitly documented as deferred targets after v1.
- Adapter commands (`/agent`, `!agent`) return configured agents for the resolved orchestrator/channel profile.
- Workflow dispatch directives never leak directly to end-user channel messages.
