#!/usr/bin/env bash
# Runs the TypeScript and the Rust extractor on the same sample of a trace-data checkout and
# diffs every output file. Only the generation timestamps may differ.
#
# Usage: TRACE_DATA_DIR=<trace-data> scripts/parity.sh [PERCENT] [TS_OUTPUT_DIR]
#
# PERCENT defaults to 1. Without TS_OUTPUT_DIR the TypeScript tool runs from a clean copy of
# HEAD in a temporary directory (it needs `npm ci` in tools/corrections-extractor once), so
# neither tool's output in the repository is touched.
set -euo pipefail

percent="${1:-1}"
ts_output="${2:-}"
: "${TRACE_DATA_DIR:?Set TRACE_DATA_DIR to a trace-data checkout}"

repo="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
rust_dir="$repo/tools/corrections-extractor-rs"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

if [ -z "$ts_output" ]; then
  echo "Running the TypeScript extractor on a $percent% sample..."
  git -C "$repo" archive HEAD tools/corrections-extractor | tar -x -C "$work"
  ln -s "$repo/tools/corrections-extractor/node_modules" "$work/tools/corrections-extractor/node_modules"
  (cd "$work/tools/corrections-extractor" && TRACE_STORE_DIR="$work/store" EXTRACT_SAMPLE="$percent" npm run -s extract >"$work/ts.log" 2>&1) ||
    { cat "$work/ts.log"; exit 1; }
  ts_output="$work/tools/corrections-extractor/output/sample-$percent"
fi

echo "Running the Rust extractor on a $percent% sample..."
cargo build --release --quiet --manifest-path "$rust_dir/Cargo.toml"
"$rust_dir/target/release/corrections-extractor-rs" --trace-data "$TRACE_DATA_DIR" --sample "$percent" --output "$work/rust" >"$work/rust.log" 2>&1 ||
  { cat "$work/rust.log"; exit 1; }

# The timestamp lines are the only expected difference.
normalize() { sed -E -e '/^-- Generated at: /d' -e '/"generatedAt": /d' "$1"; }

status=0
while IFS= read -r file; do
  if [ ! -f "$work/rust/$file" ]; then
    echo "MISSING in Rust output: $file"
    status=1
  elif ! diff -q <(normalize "$ts_output/$file") <(normalize "$work/rust/$file") >/dev/null; then
    echo "DIFFERS: $file"
    diff <(normalize "$ts_output/$file") <(normalize "$work/rust/$file") | head -20
    status=1
  fi
done < <(cd "$ts_output" && find . -type f ! -path './.*' | sed 's|^\./||' | sort)
while IFS= read -r file; do
  [ -f "$ts_output/$file" ] || { echo "EXTRA in Rust output: $file"; status=1; }
done < <(cd "$work/rust" && find . -type f ! -path './.*' | sed 's|^\./||' | sort)

if [ "$status" -eq 0 ]; then
  echo "Identical: every output file matches (timestamps aside)."
fi
exit "$status"
