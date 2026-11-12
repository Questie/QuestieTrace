#!/usr/bin/env bash
# Downloads the scraper databases the relations pipeline reads (the raw Forever and Era page
# caches) into the gitignored data/scraper/ cache, laid out as Exporters lays out its cache:
#
#   data/scraper/<tag>/manifest.json
#   data/scraper/<tag>/data/raw/<game>.db
#
# Each database is unpacked from the release's xz chunks in a temporary folder and only replaces
# anything once its size, SQLite header and SHA-256 match the release manifest. A database that
# was verified once carries a `<game>.db.verified` stamp with that checksum, so later runs reuse
# it without hashing several GB again.
#
# Usage: scripts/fetch-scraper.sh [TAG]    (default: the latest Questie/scraper release)
# Prints the release folder (what SCRAPER_DIR wants) as its last line.
# Needs gh with access to Questie/scraper, xz, jq and sha256sum.
set -euo pipefail

REPO=Questie/scraper
# Relations reads Forever's raw page cache, and Era's only to tell which claims Forever inherited.
DATABASES=(forever classic)
TYPE=raw

cache="$(cd "$(dirname "$0")/.." && pwd)/data/scraper"
tag="${1:-latest}"
if [ "$tag" = latest ]; then
  tag="$(gh release view --repo "$REPO" --json tagName --jq .tagName)"
fi
# The tag becomes a folder name and a gh argument: never a path or an option.
if ! [[ "$tag" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]] || [ "$tag" = latest ]; then
  echo "fetch-scraper: not a release tag: $tag" >&2
  exit 1
fi

release="$cache/$tag"
mkdir -p "$release/data/$TYPE"
manifest="$release/manifest.json"
if [ ! -f "$manifest" ]; then
  gh release download "$tag" --repo "$REPO" --pattern manifest.json --output "$manifest.tmp" --clobber
  mv "$manifest.tmp" "$manifest"
fi
if [ "$(jq -r .tag "$manifest")" != "$tag" ]; then
  echo "fetch-scraper: $manifest belongs to another release" >&2
  exit 1
fi

# Whether `$1` has `$2` bytes, a SQLite header and SHA-256 `$3`.
verified() {
  [ "$(stat -c %s "$1")" = "$2" ] &&
    [ "$(head -c 16 "$1" | tr -d '\0')" = "SQLite format 3" ] &&
    [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$3" ]
}

work=""
trap '[ -n "$work" ] && rm -rf "$work"' EXIT

for game in "${DATABASES[@]}"; do
  artifact="$(jq -c --arg game "$game" --arg type "$TYPE" '.versions[] | select(.game_version == $game) | .[$type]' "$manifest")"
  if [ -z "$artifact" ]; then
    echo "fetch-scraper: $tag has no $TYPE $game database" >&2
    exit 1
  fi
  sha="$(jq -r .sha256 <<<"$artifact")"
  size="$(jq -r .size_bytes <<<"$artifact")"
  db="$release/data/$TYPE/$game.db"

  if [ -f "$db" ] && [ "$(cat "$db.verified" 2>/dev/null)" = "$sha" ] && [ "$(stat -c %s "$db")" = "$size" ]; then
    echo "fetch-scraper: $tag/$TYPE/$game.db verified earlier" >&2
    continue
  fi
  if [ -f "$db" ] && verified "$db" "$size" "$sha"; then
    echo "$sha" >"$db.verified"
    echo "fetch-scraper: $tag/$TYPE/$game.db verified" >&2
    continue
  fi

  work="$(mktemp -d "$release/data/$TYPE/.$game-download-XXXXXX")"
  mapfile -t files < <(jq -r '.files[]' <<<"$artifact")
  for file in "${files[@]}"; do
    if ! [[ "$file" =~ ^[A-Za-z0-9._-]+$ ]]; then
      echo "fetch-scraper: unexpected file name in the manifest: $file" >&2
      exit 1
    fi
    echo "fetch-scraper: downloading $tag/$file" >&2
    gh release download "$tag" --repo "$REPO" --pattern "$file" --dir "$work"
  done
  echo "fetch-scraper: unpacking $game.db ($((size / 1024 / 1024)) MB)" >&2
  (cd "$work" && cat "${files[@]}") | xz -d >"$work/$game.db"
  if ! verified "$work/$game.db" "$size" "$sha"; then
    echo "fetch-scraper: $game.db failed the size, SQLite header or SHA-256 check" >&2
    exit 1
  fi
  rm -f "$db.verified"
  mv "$work/$game.db" "$db"
  echo "$sha" >"$db.verified"
  rm -rf "$work"
  work=""
  echo "fetch-scraper: $tag/$TYPE/$game.db verified" >&2
done

echo "$release"
