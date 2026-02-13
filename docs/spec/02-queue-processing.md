# Queue Processing and Message Lifecycle

## Scope

Defines queue payload contracts, file movement lifecycle, ordering guarantees, output naming, and recovery behavior.

## Queue Payload Contracts

Incoming JSON schema:

```json
{
  "channel": "discord|telegram|whatsapp|slack|heartbeat",
  "channelProfileId": "optional_channel_profile_id",
  "sender": "Display Name",
  "senderId": "channel_user_id",
  "message": "text (may include [file: /abs/path])",
  "timestamp": 1700000000000,
  "messageId": "channel_specific_unique_id",
  "conversationId": "optional_channel_conversation_or_thread_id",
  "files": ["/abs/path1", "/abs/path2"],
  "workflowRunId": "optional_workflow_run_id",
  "workflowStepId": "optional_workflow_step_id"
}
```

Outgoing JSON schema:

```json
{
  "channel": "same_channel",
  "channelProfileId": "same_channel_profile_id_when_present",
  "sender": "original sender",
  "message": "response text (send_file tags removed)",
  "originalMessage": "raw original text",
  "timestamp": 1700000005000,
  "messageId": "original message id",
  "agent": "resolved_step_agent_id",
  "conversationId": "original conversation/thread id when present",
  "files": ["/abs/path_from_send_file_tags"],
  "workflowRunId": "optional_workflow_run_id",
  "workflowStepId": "optional_workflow_step_id"
}
```

## Processing Algorithm

Queue worker behavior:

1. Read `incoming/*.json`, sorted by file `mtime` ascending.
2. Claim each item by atomic move `incoming -> processing`.
3. Resolve execution path:
   - If `workflowRunId` is present and message is an explicit status command (`status|progress|/status|/progress`, case-insensitive), return current run progress snapshot without advancing workflow steps.
   - Else if `workflowRunId` is present, dispatch to that workflow run.
   - Else dispatch to orchestrator intent-selection phase using `channel` + `channelProfileId` + `conversationId` context.
4. For new channel messages, orchestrator must:
   - resolve `orchestrator_id` from `settings.yaml` channel profile mapping
   - load `<orchestrator_private_workspace>/orchestrator.yaml`
   - write selector request file under `~/.rustyclaw/orchestrator/select/incoming`
   - run selector agent via provider CLI
   - persist selector output under `~/.rustyclaw/orchestrator/select/results`
   - execute selector-chosen action:
     - `workflow_start`: select workflow and start new run
     - `workflow_status`: resolve run in precedence order and return run progress snapshot without advancing workflow steps:
       1. `workflowRunId` when present
       2. active run association for `(channelProfileId, conversationId)`
       3. deterministic "no active workflow run found for this conversation" response when unresolved
     - `diagnostics_investigate`: gather bounded diagnostics context for the resolved run/conversation, run diagnostics agent inference, and return natural-language findings plus next-step options without advancing workflow steps
     - `command_invoke`: execute one validated function from selector `availableFunctions` with validated arguments and return command result payload
5. Orchestrator executes workflow step(s) and returns response payload fields.
6. On success, write an outgoing payload file.
7. On failure, attempt requeue `processing -> incoming`.

Output file naming:

- Heartbeat messages: `outgoing/<messageId>.json`
- All other channels: `outgoing/<channel>_<messageId>_<timestamp>.json`

## Concurrency and Ordering

Must preserve:

- FIFO claim order from incoming queue.
- Strict sequential processing per `workflowRunId` when present.
- Strict sequential processing per `(channel, channelProfileId, conversationId)` key when present.
- Parallel execution allowed across independent keys.

Cross-agent failures or delays must not block unrelated agents.

## Error and Recovery Behavior

- Queue claim/move operations must be atomic where possible.
- Processing failures must never silently drop messages.
- Requeue attempts must preserve payload content and execution eligibility.
- Worker restarts must safely tolerate partially moved queue files.

## Acceptance Criteria

- End-to-end lifecycle works: `incoming -> processing -> outgoing`.
- Channel-originated messages are dispatched to orchestrator workflow-selection then workflow execution.
- Ordering is preserved for multiple queued messages in the same conversation/workflow key.
- Messages for different keys can execute concurrently.
- Forced execution failures produce requeue attempts and logs.
