# corrections-extractor

Extracts Questie DB corrections from every QuestieTrace trace file in `Traces/` and writes them as paste-ready `forever*Traces.lua` modules.

The output is a set of CORRECTIONS modules (only fields we actually observed), meant to be layered on top of Questie's base DB, not a full DB dump. Only sessions from WoW Forever builds (interfaceVersion `16xxx`) are used, and only IDs above the known Classic maximum per entity are exported.

## Requirements

- Node.js 24
- npm

## Installation

```sh
cd tools/corrections-extractor
npm ci
```

## Usage

```sh
npm run extract
```

Every `.lua` file in `../../Traces/` is loaded, processed and dropped again one at a time, so memory use does not grow with the size of the trace files. Files that fail to load are reported and skipped; the rest still runs.

The results are written to `output/`:

| File | Content |
|---|---|
| `foreverNpcTraces.lua` | NPC corrections |
| `foreverQuestTraces.lua` | Quest corrections |
| `foreverItemTraces.lua` | Item corrections |
| `foreverObjectTraces.lua` | Object corrections |
| `meta.json` | `sessionCount`, `fileCount`, `skippedFiles` and `generatedAt` of the run |

The trace-analyzer's Extract tab shows these files for copy/download.

## How it works

1. `src/core/loader.ts` parses a trace file via `lua-state`.
2. `foldSessions` (`src/extract/index.ts`) runs every observer in `src/extract/observers/` over the file's sessions and collects their observations. Fields with the default scalar merge only keep a per-value summary; fields with a custom merge (spawns, drops, ...) keep every observation.
3. After the last file, `finalize` aggregates the observations into one fact per entity and field, builds the per-entity records (`src/extract/emit/`) and writes the Lua modules (`src/extract/schema/corrections-writer.ts`).

## Development

```sh
npm test
npm run typecheck
```
