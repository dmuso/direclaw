#!/usr/bin/env bash
set -euo pipefail

THRESHOLD_SECONDS="${1:-0.5}"
REPORT_DIR="docs/build/review/slow-tests"
TIMESTAMP="$(date '+%Y%m%d-%H%M%S')"

mkdir -p "$REPORT_DIR"

manifest="$(mktemp)"
raw_tsv="$REPORT_DIR/direclaw-test-times-$TIMESTAMP.tsv"
sorted_tsv="$REPORT_DIR/direclaw-test-times-sorted-$TIMESTAMP.tsv"
over_tsv="$REPORT_DIR/direclaw-slow-tests-over-${THRESHOLD_SECONDS}s-$TIMESTAMP.tsv"
report_md="$REPORT_DIR/test-performance-report-$TIMESTAMP.md"

echo "Compiling tests (no run)..."
cargo test --all --no-run --message-format=json > "$manifest"

mapfile -t bins < <(
  jq -r 'select(.reason=="compiler-artifact") | select(.profile.test==true) | .executable // empty' "$manifest" | sort -u
)

: > "$raw_tsv"

echo "Discovered ${#bins[@]} test binaries."
for bin in "${bins[@]}"; do
  while IFS= read -r line; do
    test_name="${line%: test}"
    [[ -z "$test_name" ]] && continue

    TIMEFORMAT=%R
    set +e
    elapsed="$({ time "$bin" --exact "$test_name" --nocapture >/tmp/direclaw-test-run.log 2>&1; } 2>&1)"
    status=$?
    set -e

    printf "%s\t%s\t%s\t%s\n" "$elapsed" "$status" "$bin" "$test_name" >> "$raw_tsv"
  done < <("$bin" --list --format terse 2>/dev/null)
done

sort -t $'\t' -k1,1nr "$raw_tsv" > "$sorted_tsv"
awk -F $'\t' -v threshold="$THRESHOLD_SECONDS" '$1+0 > threshold {print}' "$sorted_tsv" > "$over_tsv"

total_count="$(wc -l < "$raw_tsv" | tr -d '[:space:]')"
over_count="$(wc -l < "$over_tsv" | tr -d '[:space:]')"
run_timestamp="$(date '+%Y-%m-%d %H:%M:%S %Z')"

{
  echo "# DireClaw Test Performance Report"
  echo
  echo "- Run timestamp: $run_timestamp"
  echo "- Threshold: > ${THRESHOLD_SECONDS}s"
  echo "- Total timed test executions: $total_count"
  echo "- Over-threshold entries: $over_count"
  echo
  echo "## Tests Over Threshold"
  echo
  if [[ "$over_count" -eq 0 ]]; then
    echo "No tests exceeded ${THRESHOLD_SECONDS}s."
  else
    echo "| Runtime (s) | Status | Binary | Test |"
    echo "|---:|---:|---|---|"
    awk -F $'\t' '{printf "| %s | %s | `%s` | `%s` |\n", $1, $2, $3, $4}' "$over_tsv"
  fi
  echo
  echo "## Artifacts"
  echo
  echo "- Raw timings: \`$raw_tsv\`"
  echo "- Sorted timings: \`$sorted_tsv\`"
  echo "- Over-threshold timings: \`$over_tsv\`"
} > "$report_md"

echo "Done."
echo "Report: $report_md"
echo "Raw TSV: $raw_tsv"
echo "Sorted TSV: $sorted_tsv"
echo "Over-threshold TSV: $over_tsv"
