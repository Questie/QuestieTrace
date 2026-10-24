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

Every `.lua` file in `../../Traces/` is loaded, processed and dropped again one at a time, so memory use does not grow with the size of the trace files. Files that fail to load are reported and skipped; the rest still runs. A session found in more than one file (e.g. when a player exported again without confirming the previous export as reported) is only used once. The run prints a warning with the number of skipped sessions, and `meta.json` lists them with both file names.

The results are written to `output/`:

| File | Content |
|---|---|
| `foreverNpcTraces.lua` | NPC corrections |
| `foreverQuestTraces.lua` | Quest corrections |
| `foreverItemTraces.lua` | Item corrections |
| `foreverObjectTraces.lua` | Object corrections |
| `meta.json` | `sessionCount`, `fileCount`, `skippedFiles`, `duplicateSessions` and `generatedAt` of the run |

The trace-analyzer's Extract tab shows these files for copy/download.

### Translations

```sh
npm run extract:lookups
```

Writes Questie l10n lookup modules for every non-enUS locale found in the Forever traces, one directory per entity (`output/lookupNpcs/`, `output/lookupQuests/`, `output/lookupItems/`, `output/lookupObjects/`) with one file per locale (e.g. `output/lookupItems/deDE.lua`). Like the corrections, only IDs above the known Classic maximum are exported. NPC subnames are not traced and are always `nil`.

## How it works

1. `src/core/loader.ts` parses a trace file via `lua-state`.
2. `foldSessions` (`src/extract/index.ts`) runs every observer in `src/extract/observers/` over the file's sessions and collects their observations. Fields with the default scalar merge only keep a per-value summary; fields with a custom merge (spawns, drops, ...) keep every observation.
3. After the last file, `finalize` aggregates the observations into one fact per entity and field, builds the per-entity records (`src/extract/emit/`) and writes the Lua modules (`src/extract/schema/corrections-writer.ts`).

### Quest-giver correlation

`startedBy` and `finishedBy` prefer a valid `questnpc` giver, using `npc` only as a fallback. NPC/object `questStarts` and `questEnds` are derived from those exported quest records so both sides agree.

Quest and `GOSSIP_SHOW` lookups include synchronous API samples recorded after the raw event, but stop before the next raw event in the same frame. Ambiguous clock ties are skipped rather than resolved to an arbitrary giver. Recorded nil transitions clear earlier GUIDs; unchanged GUIDs remain usable during long dialogs.

Gossip lists remain change-only. The extractor replays every `GOSSIP_SHOW` using the last observed available/active quest lists and the giver at that event. This recovers identical lists offered by different NPCs without changing recordings, including existing traces. An observed empty or nil list clears previous quests. No gossip relation is inferred without a `GOSSIP_SHOW` event.

As with GUID replay, missing samples retain the last observed state. The format cannot distinguish an unchanged successful read from a failed probe. A failed list read at a different NPC can therefore reuse an earlier list incorrectly; this is an explicit limitation of last-observed-state inference, not proof of a successful read at every event.

## Development

```sh
npm test
npm run typecheck
```
