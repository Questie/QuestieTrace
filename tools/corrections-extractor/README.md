# corrections-extractor

Turns QuestieTrace traces into paste-ready Questie DB corrections (`forever*Traces.lua` modules) for WoW Forever.

The output is a set of CORRECTIONS modules (only fields we actually observed), meant to be layered on top of Questie's base DB, not a full DB dump. Only sessions from WoW Forever builds (interfaceVersion `16xxx`) are used.

The package holds two pipelines that share their input and output conventions but not their inference code:

| Pipeline | Command | Infers | Code |
|---|---|---|---|
| Field extractor | `npm run extract` | Per-entity facts: names, levels, spawns, quest givers, drops, objectives | `src/extract/` |
| Quest relations | `npm run relations -- pipeline` | Relationships between quests: prerequisites, exclusivity, chains, breadcrumbs | `src/relations/` ([README](src/relations/README.md)) |

Code both pipelines use lives in `src/core/`: trace types, the trace readers, the GUID parser and the corrections writer. `src/extract/` and `src/relations/` never import each other.

## Requirements

- Node.js 24
- npm
- Lua 5.1 (only for `npm run relations -- catalog`, which runs QuestieDB's own loader)

## Installation

```sh
cd tools/corrections-extractor
npm ci
```

## Inputs

| Input | Used by | How to get it |
|---|---|---|
| trace-data submissions | both | Clone `Questie/trace-data` into `../../trace-data` (or set `TRACE_DATA_DIR`) and `git pull` for new submissions |
| Decoded trace files in `../../Traces/` | field extractor (default mode), trace-analyzer | `tools/decoder/run.sh` |
| QuestieDB checkout | quest relations | Clone `Questie/QuestieDB` next to this repo (or set `QUESTIEDB_DIR`) |
| Wowhead scrape (`scraper-questie/data/raw/forever.db`) | quest relations, review context only | `uv run python download.py --version forever --type raw` in `Questie/scraper` (about 6 GB), or set `SCRAPER_DIR` |
| DB2 tables (AreaTable, QuestSort, UiMap) | field extractor | Committed under `src/extract/resources/` for build 1.60.1.70009; refreshed by hand from wago.tools |

## Usage

```sh
npm run extract                          # every decoded trace file in ../../Traces/
npm run extract -- --trace-data [dir]    # every submission in a trace-data checkout
npm run relations -- pipeline            # quest relations, from trace-data through combine
```

Without a flag, every `.lua` file in `../../Traces/` is loaded, processed and dropped again one at a time, so memory use does not grow with the size of the trace files. Files that fail to load are reported and skipped; the rest still runs.

`--trace-data` reads trace-data submissions directly, without the Lua decoder step. `dir` defaults to `TRACE_DATA_DIR`, else `../../trace-data`. Exports are cumulative, so one recording session appears in many submissions: submissions are read newest first and only the first (newest, most complete) copy of each session is folded in.

The relations pipeline caches its intermediate files in `.relations/` (gitignored); see its [README](src/relations/README.md) for the individual steps.

## Output

Both pipelines write their deliverables to `output/` (gitignored). Each one only ever replaces its own files, so they can run in any order.

| File | Written by | Content |
|---|---|---|
| `foreverNpcTraces.lua` | `npm run extract` | NPC corrections |
| `foreverQuestTraces.lua` | `npm run extract` | Quest corrections |
| `foreverItemTraces.lua` | `npm run extract` | Item corrections |
| `foreverObjectTraces.lua` | `npm run extract` | Object corrections |
| `meta.json` | `npm run extract` | `sessionCount`, `fileCount`, `skippedFiles`, `generatedAt`, plus `source` (`traces` or `trace-data`), `submissionCount` and `duplicateSessionsSkipped`. In trace-data mode `fileCount` counts decoded exports |
| `foreverQuestRelationTraces.lua` | `npm run relations -- combine` | Quest relationship fields only (module `ForeverQuestRelationTraces`) |
| `relations-review.md` | `npm run relations -- combine` | Evidence for every accepted relation, near misses and conflicts. Read it before trusting the module |
| `relations-meta.json` | `npm run relations -- combine` | Summary of the relations run |

The trace-analyzer's Extract tab shows all of them for copy/download; the relations files are under "Quest relations". Shipping the relations module also needs a QuestieDB change: copy it into `src/corrections/Forever/traces/` and register `ForeverQuestRelationTraces` there.

## How the field extractor works

1. Trace sessions come from `src/core/loader.ts` (decoded `.lua` files via `lua-state`) or, with `--trace-data`, from `src/core/trace-data/` (`submissions.ts`, `decode.ts`, `cbor.ts`; `session-key.ts` identifies copies of one session).
2. `foldSessions` (`src/extract/index.ts`) runs every observer in `src/extract/observers/` over the sessions and collects their observations. Fields with the default scalar merge only keep a per-value summary; fields with a custom merge (spawns, drops, ...) keep every observation.
3. After the last file, `finalize` aggregates the observations into one fact per entity and field, builds the per-entity records (`src/extract/emit/`) and writes the Lua modules (`src/core/corrections-writer.ts`). Only IDs above the known Classic maximum per entity (`MAX_IDS` in `src/extract/index.ts`) are exported.

## Development

```sh
npm test
npm run typecheck
DECODE_ORACLE=1 npx vitest run src/core/trace-data/decode.oracle.test.ts   # compare the native decoder with tools/decoder (needs lua5.1 and trace-data)
```
