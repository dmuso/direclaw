#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

TASK_DIR="docs/build/tasks"
SPEC="docs/build/spec/"

if ! command -v codex >/dev/null 2>&1; then
  echo "Error: 'codex' CLI is not installed or not in PATH." >&2
  exit 1
fi

if [[ ! -d "$TASK_DIR" ]]; then
  echo "Error: task directory not found: $TASK_DIR" >&2
  exit 1
fi

task_files=()
while IFS= read -r task_file; do
  task_files+=("$task_file")
done < <(
  find "$TASK_DIR" -maxdepth 1 -type f \
    \( -name 'phase-*.md' -o -name 'task-*.md' \) \
    ! -name '*-review.*' \
    | sort
)

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

  echo "  implement tasks from '$x'"
  codex exec --yolo "Implement all tasks described in file '$x'."

  echo "  review work '$x', output report to '$y'"
  codex exec --yolo "Review the uncommitted work (or the last commit if there are no uncommitted changes) from task file '$x' against the spec doc in '$SPEC'. Write a review for anything that needs actioning to '$y'."

  echo "  action the review items from '$y'"
  codex exec --yolo "Rectify all issues identified in review doc '$y' and then commit all uncommitted work."

  echo "=== Completed task: $x ==="
  echo

done
