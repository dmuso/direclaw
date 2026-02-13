#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

TASK_DIR="docs/tasks"
SPEC_DIR="./docs/spec"

if ! command -v codex >/dev/null 2>&1; then
  echo "Error: 'codex' CLI is not installed or not in PATH." >&2
  exit 1
fi

if [[ ! -d "$TASK_DIR" ]]; then
  echo "Error: task directory not found: $TASK_DIR" >&2
  exit 1
fi

mapfile -t task_files < <(find "$TASK_DIR" -maxdepth 1 -type f ! -name '*-review.*' | sort)

if [[ ${#task_files[@]} -eq 0 ]]; then
  echo "No task files found in $TASK_DIR"
  exit 0
fi

for x in "${task_files[@]}"; do
  dir_name="$(dirname "$x")"
  base_name="$(basename "$x")"

  if [[ "$base_name" == *.* ]]; then
    file_stem="${base_name%.*}"
    file_ext=".${base_name##*.}"
  else
    file_stem="$base_name"
    file_ext=""
  fi

  y="$dir_name/${file_stem}-review${file_ext}"

  echo "=== Processing task: $x ==="

  codex --non-interactive "Implement all tasks described in file '$x'."

  codex --non-interactive "Review the uncommitted work from task file '$x' against the spec docs in '$SPEC_DIR'. Write a report to '$y'."

  codex --non-interactive "Rectify all issues identified in review doc '$y' and then commit all uncommitted work. Include in the commit message a comprehensive feature based description of every change in the commit."

  echo "=== Completed task: $x ==="
  echo

done
