#!/usr/bin/env bash
# Runs the quest-relations pipeline (crates/relations) after checking every input, so a run stops
# at once with the reason instead of halfway through. The scraper databases are fetched into the
# gitignored data/scraper/ cache (scripts/fetch-scraper.sh) unless SCRAPER_DIR is given.
#
# Usage: QUESTIEDB_DIR=<QuestieDB checkout> scripts/relations.sh [--twice] [RELATIONS_DIR]
#
#   QUESTIEDB_DIR       required: a QuestieDB checkout with master's corrections layout
#   TRACE_DATA_DIR      trace-data checkout (default: trace-data next to the QuestieTrace checkout)
#   SCRAPER_DIR         a scraper release folder (default: fetch SCRAPER_TAG into data/scraper/)
#   SCRAPER_TAG         release to fetch (default: the latest)
#   RELATIONS_TRACE_MODULES_DIR
#                       optional: a dump's own forever*Traces.lua for the catalog; without it the
#                       catalog uses the trace modules imported into QUESTIEDB_DIR
#   RELATIONS_DIR       where the run's files go (default: relations-output/ here, gitignored);
#                       the deliverables end up in its output/
#   --twice             run the pipeline twice and compare the two runs: a flipped bit (this
#                       machine has shown memory errors) shows up as a difference
set -euo pipefail

tool="$(cd "$(dirname "$0")/.." && pwd)"
twice=false
if [ "${1:-}" = --twice ]; then
  twice=true
  shift
fi
out="$(realpath -m "${1:-$tool/relations-output}")"

fail() {
  echo "relations: $*" >&2
  exit 1
}
note() { echo "relations: $*" >&2; }

# --- trace-data -------------------------------------------------------------------------------
trace_data="$(realpath -m "${TRACE_DATA_DIR:-$tool/../../../trace-data}")"
[ -d "$trace_data/submissions" ] || fail "no submissions/ in $trace_data (set TRACE_DATA_DIR)"
submissions="$(find -L "$trace_data/submissions" -mindepth 2 -maxdepth 2 -name '*.json' | wc -l)"
[ "$submissions" -gt 0 ] || fail "no submissions in $trace_data/submissions"
note "trace-data: $submissions submissions in $trace_data"

# --- QuestieDB --------------------------------------------------------------------------------
[ -n "${QUESTIEDB_DIR:-}" ] || fail "set QUESTIEDB_DIR to a QuestieDB checkout (master's layout)"
questie_db="$(realpath -m "$QUESTIEDB_DIR")"
[ -f "$questie_db/generator/flavor.lua" ] || fail "$questie_db is no QuestieDB checkout (no generator/flavor.lua)"
# The catalog layers legacy -> generated -> traces -> authored; without the generated base (the
# restructured `forever` branch) it would lose nearly every Forever-new quest.
[ -f "$questie_db/src/corrections/Forever/generated/foreverBaseQuest.lua" ] ||
  fail "$questie_db has no generated Forever base: check out master (the \`forever\` branch's layout is not supported)"
if git -C "$questie_db" rev-parse --git-dir >/dev/null 2>&1; then
  changed="$(git -C "$questie_db" status --porcelain --untracked-files=no | wc -l)"
  note "QuestieDB: $(git -C "$questie_db" rev-parse --abbrev-ref HEAD) at $(git -C "$questie_db" log -1 --format='%h %cs')$([ "$changed" -gt 0 ] && echo ", $changed changed tracked files")"
fi
if [ -x "$questie_db/tools/lua-binary/linux-x64/lua" ]; then
  note "Lua: QuestieDB's bundled interpreter"
elif command -v lua5.1 >/dev/null; then
  note "Lua: $(command -v lua5.1)"
else
  fail "no Lua 5.1: QuestieDB's tools/lua-binary/linux-x64/lua is missing and lua5.1 is not on PATH"
fi

# The catalog reads either a dump's own trace modules or the ones imported into QuestieDB. If an
# earlier relations output was imported there, this run would partly build on its own output.
relation_fields='questKeys\.(preQuestSingle|preQuestGroup|exclusiveTo|nextQuestInChain|parentQuest|breadcrumbForQuestId|breadcrumbs|availableUntilCompleted|availableStartingWith|disabledByQuest)\]'
if [ -n "${RELATIONS_TRACE_MODULES_DIR:-}" ]; then
  modules="$(realpath -m "$RELATIONS_TRACE_MODULES_DIR")"
  ls "$modules"/forever*Traces.lua >/dev/null 2>&1 || fail "no forever*Traces.lua in RELATIONS_TRACE_MODULES_DIR=$modules"
  note "trace modules: $modules"
else
  modules="$questie_db/src/corrections/Forever/traces"
  note "trace modules: the ones imported into QuestieDB"
fi
if grep -qE "$relation_fields" "$modules"/forever*Traces.lua 2>/dev/null; then
  note "WARNING: $modules already holds relation fields (an imported relations output?); the catalog will treat them as known"
fi

# --- scraper ----------------------------------------------------------------------------------
if [ -n "${SCRAPER_DIR:-}" ]; then
  scraper="$(realpath -m "$SCRAPER_DIR")"
else
  scraper="$("$tool/scripts/fetch-scraper.sh" "${SCRAPER_TAG:-latest}" | tail -1)"
fi
for game in forever classic; do
  [ -f "$scraper/data/raw/$game.db" ] || fail "no data/raw/$game.db in $scraper (set SCRAPER_DIR or unset it to fetch one)"
done
note "scraper: $scraper"

# --- memory -----------------------------------------------------------------------------------
available_gb=$(($(awk '/MemAvailable/ {print $2}' /proc/meminfo) / 1024 / 1024))
[ "$available_gb" -ge 16 ] || note "WARNING: $available_gb GB of memory available; the full archive needs about 15 GB"

# --- run --------------------------------------------------------------------------------------
cargo build --release --quiet --manifest-path "$tool/Cargo.toml" -p relations

run() {
  mkdir -p "$1"
  note "running the pipeline into $1"
  # Through env: the optional trace-modules variable is an expansion, which a plain prefix
  # assignment would not take.
  env TRACE_DATA_DIR="$trace_data" QUESTIEDB_DIR="$questie_db" SCRAPER_DIR="$scraper" RELATIONS_DIR="$1" \
    RELATIONS_REQUIRE_FOREVER_VERSION=1 RELATIONS_OUTPUT_DIR="$1/output" \
    ${RELATIONS_TRACE_MODULES_DIR:+RELATIONS_TRACE_MODULES_DIR="$modules"} \
    "$tool/target/release/relations" pipeline
}

run "$out"
if $twice; then
  run "$out-2"
  if "$tool/scripts/relations-compare.sh" "$out" "$out-2"; then
    note "both runs give the same values"
  else
    fail "the two runs differ: a bit flipped in one of them; run again before trusting either"
  fi
fi
note "deliverables: $out/output/ (foreverQuestRelationTraces.lua, relations-review.md, relations-meta.json)"
