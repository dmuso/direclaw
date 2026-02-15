#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

SPEC="docs/build/domain-modelling-file-structure-proposal.md"
LOOP_COUNT="${1:-${LOOP_COUNT:-20}}"
INTERRUPTED=0

on_interrupt() {
  INTERRUPTED=1
}

trap on_interrupt INT

if ! command -v codex >/dev/null 2>&1; then
  echo "Error: 'codex' CLI is not installed or not in PATH." >&2
  exit 1
fi

if ! [[ "$LOOP_COUNT" =~ ^[0-9]+$ ]] || [[ "$LOOP_COUNT" -lt 1 ]]; then
  echo "Error: loop count must be a positive integer. Got: '$LOOP_COUNT'" >&2
  exit 1
fi

for ((i = 1; i <= LOOP_COUNT; i++)); do
  if ((INTERRUPTED)); then
    echo "Interrupted. Exiting."
    break
  fi

  echo "=== Continuing refactor ($i/$LOOP_COUNT) ==="
  status=0
  codex exec --yolo "We are iteratively refactoring to a better domain model which requires splitting files and moving/grouping related functgionality. The end goal of our domain structure is documented in '$SPEC'. This is a living, working document. At the bottom of the document there is a running log of iterative changes that have been made so far. Continue this work. Analyse the document, the current file structure, and the running log and identify the next logical change to move us one step closer to the desired outcome. Ensure all tests pass once your change is made. Update the running log and add an entry for your work once done. Commit your work as the last step." || status=$?

  if ((INTERRUPTED)) || [[ "$status" -eq 130 ]]; then
    echo "Interrupted. Exiting."
    break
  fi

  if [[ "$status" -ne 0 ]]; then
    exit "$status"
  fi
done
