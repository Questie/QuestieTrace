#!/usr/bin/env bash
# Extracts export strings from trace-data submissions, decodes them, and
# moves the resulting Lua traces into QuestieTrace/Traces.
#
# Usage:
#   ./run.sh [trace-data-dir]

set -euo pipefail

scriptDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tracesDir="$scriptDir/../../Traces"

if ! command -v lua5.1 >/dev/null 2>&1; then
  echo "error: lua5.1 is required but was not found on PATH" >&2
  exit 1
fi

lua5.1 "$scriptDir/extract.lua" "$@"
lua5.1 "$scriptDir/decoder.lua"

mkdir -p "$tracesDir"
shopt -s nullglob
outputFiles=("$scriptDir"/output/*.lua)
if [ ${#outputFiles[@]} -eq 0 ]; then
  echo "no decoded files to move"
else
  # decoder.lua emits "return { ... }" (a plain Lua table for reading/diffing).
  # Traces/ needs real SavedVariables-style files, i.e. "QuestieTraceCharacter
  # = { ... }", which is what tools/trace-analyzer expects as a global.
  for f in "${outputFiles[@]}"; do
    sed '1s/^return {/QuestieTraceCharacter = {/' "$f" > "$f.tmp" && mv -f "$f.tmp" "$f"
  done
  mv -f "${outputFiles[@]}" "$tracesDir/"
  echo "moved ${#outputFiles[@]} file(s) to '$tracesDir'"
fi
