#!/usr/bin/env bash
# Compares every file of a relations run with another run's, by value (see
# crates/relations/examples/compare.rs): the gate between the Rust and the TypeScript pipeline.
#
# Usage: scripts/relations-compare.sh EXPECTED_RELATIONS_DIR ACTUAL_RELATIONS_DIR
# Both are RELATIONS_DIR folders; deliverables are expected in output/ under each.
set -euo pipefail

expected="$1"
actual="$2"
cd "$(dirname "$0")/.."
cargo build --release --quiet -p relations --example compare
compare=target/release/examples/compare

status=0
while IFS= read -r file; do
  if [ ! -f "$actual/$file" ]; then
    echo "MISSING: $file"
    status=1
  elif ! result=$("$compare" "$expected/$file" "$actual/$file" --max 5); then
    echo "DIFFERS: $file"
    echo "$result" | sed 's/^/  /'
    status=1
  fi
done < <(cd "$expected" && find . -type f \( -name '*.json' -o -name '*.jsonl' -o -name '*.lua' -o -name '*.md' \) ! -path './ingest-cache/*' | sed 's|^\./||' | sort)

[ "$status" -eq 0 ] && echo "Same values: every file matches."
exit "$status"
