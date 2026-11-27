#!/usr/bin/env bash
# Times two builds of the extractor back to back on the same sample, alternating, so slow drifts
# of the machine hit both alike. Prints one line per run and the median of each side.
#
# Usage: TRACE_DATA_DIR=<trace-data> scripts/ab.sh BASELINE_BIN CANDIDATE_BIN [PERCENT] [ROUNDS]
set -euo pipefail

baseline="$1"
candidate="$2"
percent="${3:-5}"
rounds="${4:-2}"
: "${TRACE_DATA_DIR:?Set TRACE_DATA_DIR to a trace-data checkout}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# One line per run: elapsed seconds, user, sys, peak RSS in MB.
measure() {
  /usr/bin/time -f "%e %U %S %M" -o "$work/time" "$1" --trace-data "$TRACE_DATA_DIR" --sample "$percent" --output "$work/out" >/dev/null 2>&1
  read -r elapsed user sys rss <"$work/time"
  printf '%-9s %7.1f s elapsed %8.0f s user %6.0f s sys %7.0f MB peak\n' "$2" "$elapsed" "$user" "$sys" "$((rss / 1024))"
  echo "$elapsed $((rss / 1024))" >>"$work/$2"
}

for _ in $(seq "$rounds"); do
  measure "$baseline" baseline
  measure "$candidate" candidate
done

median() { sort -n -k1 "$work/$1" | awk '{a[NR]=$1; m[NR]=$2} END {i=int((NR+1)/2); printf "%.1f s, %d MB", a[i], m[i]}'; }
echo "median: baseline $(median baseline), candidate $(median candidate)"
