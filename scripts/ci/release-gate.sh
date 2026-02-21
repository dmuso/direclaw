#!/usr/bin/env bash
set -euo pipefail

ARTIFACTS_DIR=""
TESTS_MARKER=""
DOCS_MARKER=""
TRACEABILITY_FILE="docs/build/review/requirement-traceability.md"
CHECKLIST_FILE="docs/build/release-checklist.md"
RELEASE_TAG=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifacts-dir)
      ARTIFACTS_DIR="$2"
      shift 2
      ;;
    --tests-marker)
      TESTS_MARKER="$2"
      shift 2
      ;;
    --docs-marker)
      DOCS_MARKER="$2"
      shift 2
      ;;
    --traceability-file)
      TRACEABILITY_FILE="$2"
      shift 2
      ;;
    --checklist-file)
      CHECKLIST_FILE="$2"
      shift 2
      ;;
    --release-tag)
      RELEASE_TAG="$2"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$ARTIFACTS_DIR" || -z "$TESTS_MARKER" || -z "$DOCS_MARKER" ]]; then
  echo "usage: $0 --artifacts-dir <dir> --tests-marker <file> --docs-marker <file> [--traceability-file <file>] [--checklist-file <file>] [--release-tag <tag>]" >&2
  exit 2
fi

fail() {
  echo "release_gate=fail"
  echo "reason=$1"
  exit 1
}

[[ -s "$TESTS_MARKER" ]] || fail "tests gate marker missing or empty: $TESTS_MARKER"
[[ -s "$DOCS_MARKER" ]] || fail "docs gate marker missing or empty: $DOCS_MARKER"
[[ -f "$CHECKLIST_FILE" ]] || fail "release checklist missing: $CHECKLIST_FILE"
[[ -f "$TRACEABILITY_FILE" ]] || fail "traceability file missing: $TRACEABILITY_FILE"

for rb in RB-1 RB-2 RB-3 RB-4 RB-5 RB-6 RB-7; do
  if ! grep -q "$rb" "$CHECKLIST_FILE"; then
    fail "release checklist missing $rb section"
  fi
done

if grep -n "planned:" "$TRACEABILITY_FILE" >/dev/null; then
  fail "traceability still contains planned placeholder references"
fi

[[ -d "$ARTIFACTS_DIR" ]] || fail "artifacts directory missing: $ARTIFACTS_DIR"
[[ -s "$ARTIFACTS_DIR/checksums.txt" ]] || fail "checksums.txt missing or empty in $ARTIFACTS_DIR"

TARGETS=(
  "aarch64-apple-darwin"
  "x86_64-unknown-linux-musl"
  "aarch64-unknown-linux-musl"
)

for target in "${TARGETS[@]}"; do
  if [[ -n "$RELEASE_TAG" ]]; then
    expected="$ARTIFACTS_DIR/direclaw-${RELEASE_TAG}-${target}.tar.gz"
    [[ -s "$expected" ]] || fail "missing artifact for target $target: $expected"
  else
    if ! compgen -G "$ARTIFACTS_DIR/direclaw-*-${target}.tar.gz" >/dev/null; then
      fail "missing artifact for target $target in $ARTIFACTS_DIR"
    fi
  fi
done

if [[ -f "$ARTIFACTS_DIR/release-notes.md" ]] && grep -n "{{" "$ARTIFACTS_DIR/release-notes.md" >/dev/null; then
  fail "release notes contain unresolved placeholders"
fi

echo "release_gate=ok"
echo "artifacts_dir=$ARTIFACTS_DIR"
echo "tests_marker=$TESTS_MARKER"
echo "docs_marker=$DOCS_MARKER"
