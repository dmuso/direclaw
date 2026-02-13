# File Exchange and Attachment Semantics

## Scope

Defines how inbound file references and outbound send-file directives are represented and transformed.

## Inbound File Rules

- Inbound media must be stored as absolute paths.
- Queued message text includes `[file: /abs/path]` tags for media references.
- Incoming payload `files[]` may include same absolute paths for explicit machine use.

## Outbound File Rules

Assistant output may include `[send_file: /absolute/path]` tags.

Processing requirements:

- Extract all valid send-file tags into outgoing payload `files[]`.
- Remove send-file tags from outgoing user-visible text.
- Preserve non-tag text content order.

Delivery requirement:

- Channel adapters must send files before sending final text.

## Outbound Text Truncation

After send-file tag stripping:

- Hard truncate response to max 4000 chars
- Keep first 3900 chars
- Append `\n\n[Response truncated...]`

## Validation and Safety

- Only absolute file paths are valid for `[file: ...]` and `[send_file: ...]`.
- Invalid or unreadable outbound send-file paths must be logged and omitted from file send list.
- File tag parsing must be deterministic and test-covered.

## Acceptance Criteria

- File tags round-trip correctly from inbound adapter -> queue -> outbound adapter.
- Tag stripping never leaks raw send-file directives to user-visible text.
- Truncation behavior matches exact length and suffix contract.

