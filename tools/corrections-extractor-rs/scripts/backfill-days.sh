#!/usr/bin/env bash
# Packs the old archive's submissions that are in no pack yet, one day received at a time, for a
# run that is left alone for hours. Started again, it continues where it stopped.
#
# It reads the old bucket (list and download only), writes under OUT, and deletes nothing. Each
# day is a process of its own, so memory starts over with every day.
#
# Usage: BIN=... OUT=... ALSO_PACKED=... RCLONE_CONF=... scripts/backfill-days.sh FIRST_DAY LAST_DAY
#   FIRST_DAY, LAST_DAY  such as 2026-11-13; the days are those of the submissions' names, in UTC
#   BIN                  the trace-compactor binary
#   OUT                  where the packs go, OUT/packs/backfill/...; the log is OUT/backfill-days.log
#   ALSO_PACKED          a directory whose packs count as packed
#   RCLONE_CONF          an rclone config with one S3 remote, whose key may read the old bucket
#   BUCKET               the old bucket (default trace-intake-ovh)
#   MB_PER_SECOND        download limit in megabytes a second (default 40)
#   CPUS                 the CPUs the run may use, as taskset takes them (default: 8 cores). This is
#                        what bounds its load, whatever THREADS is
#   THREADS              how many submissions are downloaded and built at once (default 8). More than
#                        CPUS is of use: one thread downloads about 1.5 MB/s of small objects, and
#                        waits while it does
#   BUILD_INPUT_MIB      the most JSON being downloaded and built at once (default 64): memory
#                        follows it, at about 22 times as much
#   TP_SCHEMA            schemaVersions whose tp is rounded: those a parity run has covered
#   MIN_FREE_GIB         stops when OUT's disk has less free (default 25)
#   MIN_MEMORY_GIB       waits when less memory is available (default 6)
#   RETRY_SECONDS        the pause before a failed day is tried again (default 300)
set -uo pipefail

first="${1:?FIRST_DAY}"
last="${2:?LAST_DAY}"
: "${BIN:?}" "${OUT:?}" "${ALSO_PACKED:?}" "${RCLONE_CONF:?}"
bucket="${BUCKET:-trace-intake-ovh}"
mb_per_second="${MB_PER_SECOND:-40}"
cpus="${CPUS:-0,2,4,6,8,10,12,14}"
tp_schema="${TP_SCHEMA:-9,10,11,12,13,14,15,16,17,18}"
min_free_gib="${MIN_FREE_GIB:-25}"
min_memory_gib="${MIN_MEMORY_GIB:-6}"
threads="${THREADS:-8}"
build_input_mib="${BUILD_INPUT_MIB:-64}"
attempts=5

# One at a time on this machine, held on a Linux file system: OUT may be a Windows drive.
exec 9> "${TMPDIR:-/tmp}/backfill-days.lock"
flock -n 9 || { echo "another backfill-days.sh is running" >&2; exit 1; }

mkdir -p "$OUT"
log="$OUT/backfill-days.log"
last_attempt="$OUT/backfill-days-last-attempt.log"
note() { echo "$(date '+%m-%d %H:%M:%S') $*" | tee -a "$log"; }
free_gib() { df --output=avail -BG "$OUT" | tail -1 | tr -dc 0-9; }
memory_gib() { awk '/MemAvailable/ {print int($2 / 1048576)}' /proc/meminfo; }

# The key goes into the environment and never onto a command line.
setting() { awk -F' *= *' -v name="$1" '$1 == name {print $2; exit}' "$RCLONE_CONF"; }
SOURCE_S3_ENDPOINT="$(setting endpoint | sed -E 's#^https?://##; s#/+$##')"
SOURCE_S3_REGION="$(setting region)"
SOURCE_S3_BUCKET="$bucket"
SOURCE_S3_ACCESS_KEY_ID="$(setting access_key_id)"
SOURCE_S3_SECRET_ACCESS_KEY="$(setting secret_access_key)"
export SOURCE_S3_ENDPOINT SOURCE_S3_REGION SOURCE_S3_BUCKET SOURCE_S3_ACCESS_KEY_ID SOURCE_S3_SECRET_ACCESS_KEY
for name in SOURCE_S3_ENDPOINT SOURCE_S3_REGION SOURCE_S3_ACCESS_KEY_ID SOURCE_S3_SECRET_ACCESS_KEY; do
  [ -n "${!name}" ] || { note "$RCLONE_CONF has no value for $name"; exit 1; }
done

# One day, tried again after a failure: 0 packed, 1 gave up, 2 packed except what is reported.
pack_day() {
  local day="$1" attempt status
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    if [ "$(free_gib)" -lt "$min_free_gib" ]; then
      note "STOPPED: less than $min_free_gib GiB free for $OUT"
      exit 1
    fi
    while [ "$(memory_gib)" -lt "$min_memory_gib" ]; do
      note "waiting: less than $min_memory_gib GiB of memory available"
      sleep 120
    done
    note "day $day, attempt $attempt, $(free_gib) GiB free"
    : > "$last_attempt"
    taskset -c "$cpus" nice -n 19 ionice -c 3 "$BIN" backfill --out "$OUT" --also-packed "$ALSO_PACKED" \
      --only "${day:0:7}/$day" --tp-schema "$tp_schema" --download-mb-per-second "$mb_per_second" \
      --threads "$threads" --build-input-mib "$build_input_mib" --pack-input-mib 2048 2>&1 | tee -a "$log" "$last_attempt" > /dev/null
    status="${PIPESTATUS[0]}"
    [ "$status" -eq 0 ] && return 0
    # Submissions that could not be packed: trying again finds the same.
    grep -q '^NOT PACKED' "$last_attempt" && return 2
    note "day $day failed with status $status: $(tail -1 "$last_attempt" | cut -c1-200)"
    [ "$attempt" -lt "$attempts" ] && sleep "${RETRY_SECONDS:-300}"
  done
  return 1
}

problems=0
day="$first"
while [[ ! "$day" > "$last" ]]; do
  pack_day "$day"
  case $? in
    0) note "day $day done" ;;
    2) note "day $day done, with submissions that could not be packed: see above"; problems=$((problems + 1)) ;;
    *) note "day $day GAVE UP after $attempts attempts"; problems=$((problems + 1)) ;;
  esac
  day="$(date -u -d "$day + 1 day" +%F)"
done
note "FINISHED $first to $last, $problems days with problems"
[ "$problems" -eq 0 ]
