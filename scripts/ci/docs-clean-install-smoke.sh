#!/usr/bin/env bash
set -euo pipefail

BIN_PATH="${1:-}"
if [[ -z "$BIN_PATH" ]]; then
  echo "usage: $0 <path-to-direclaw-binary>" >&2
  exit 2
fi
if [[ ! -x "$BIN_PATH" ]]; then
  echo "binary is not executable: $BIN_PATH" >&2
  exit 2
fi

TMP_ROOT="$(mktemp -d)"
TMP_HOME="$TMP_ROOT/home"
TMP_BIN="$TMP_ROOT/bin"
mkdir -p "$TMP_HOME" "$TMP_BIN"
cp "$BIN_PATH" "$TMP_BIN/direclaw"
chmod +x "$TMP_BIN/direclaw"

export HOME="$TMP_HOME"
export PATH="$TMP_BIN:${PATH}"

# Match documented first-run flow from docs/user-guide/README.md.
direclaw setup >/dev/null
if ! direclaw orchestrator list | grep -qx "main"; then
  direclaw orchestrator add main >/dev/null
fi
direclaw channel-profile add slack_main slack main \
  --slack-app-user-id U0123456789 \
  --require-mention-in-channels true >/dev/null

direclaw start >/dev/null
direclaw status >/dev/null

# Queue a message through a configured profile as a first-message smoke check.
SEND_OUTPUT="$(direclaw send slack_main "docs smoke message")"
MESSAGE_ID="$(printf "%s\n" "$SEND_OUTPUT" | awk -F= '/^message_id=/{print $2}')"
if [[ -z "$MESSAGE_ID" ]]; then
  echo "expected message_id in send output" >&2
  exit 1
fi
if ! find "$HOME/.direclaw/queue" -maxdepth 2 -type f -name "${MESSAGE_ID}.json" | grep -q .; then
  echo "expected queued file for message_id=$MESSAGE_ID in queue directories" >&2
  exit 1
fi

direclaw stop >/dev/null
sleep 1

echo "docs_clean_install_smoke=ok"
echo "home=$HOME"
echo "message_id=$MESSAGE_ID"
