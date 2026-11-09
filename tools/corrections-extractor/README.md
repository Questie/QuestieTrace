# corrections-extractor

Extracts Questie DB corrections and translations from every submission in the OVH archive bucket (or a local [trace-data](https://github.com/Questie/trace-data) checkout) and writes them as paste-ready `forever*Traces.lua` and lookup modules.

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

Submissions come from one of two sources:

- `TRACE_REMOTE`, an rclone remote holding the archive bucket, e.g. `TRACE_REMOTE=ovhcloud-de:trace-intake-ovh npm run extract`. Only the submissions not observed yet are downloaded, 500 at a time, into a temporary directory that is removed after each batch.
- Otherwise `TRACE_DATA_DIR`, or by default the `trace-data` checkout next to the QuestieTrace checkout (`../../../trace-data`).

Every new `submissions/<yyyy-mm>/*.json` file is decoded natively (print encoding, raw deflate, CBOR) and observed in worker threads (one per CPU core but one). Set `EXTRACT_WORKERS` to use fewer threads, e.g. `EXTRACT_WORKERS=4 npm run extract` (`0` observes on the main thread). Submissions that fail to load are reported and skipped; the rest still runs.

### Observation store

The observations are kept between runs in `TRACE_STORE_DIR`, by default `trace-session-store` next to the QuestieTrace checkout (`../../../trace-session-store`):

| File | Content |
|---|---|
| `manifest.json` | Observer version and, per observed submission, its batch, load errors and counts |
| `batches/<n>.sessions.parquet` | One row per observed session copy (submission, session key, contributor, server time) |
| `batches/<n>.observations.parquet` | One row per observation (field, entity, value as JSON, confidence, time) |

A run therefore only observes submissions added since the last one. Submissions no longer in the source (rejected ones are deleted from the bucket) are dropped, and older submissions that may hold copies of their sessions are observed again. When the observer code changes (a hash of the sources listed in `src/store/observer-version.ts`), the store is dropped and every submission is observed again. Delete the directory to force that.

Each run also writes `sessions.parquet` (the sessions in use) and `views.sql` into the store, so it can be queried with the DuckDB CLI, see [the data flow](../../docs/data-flow.md#querying-observations).

### Developing observers on a sample

Changing observer code empties the store, so the next run observes every submission again. While working on an observer, use a fixed sample instead:

```sh
EXTRACT_SAMPLE=5 npm run extract   # 5 % of submissions, the same ones every run
```

The sample has its own store (`trace-session-store/sample-5/`) and output (`output/sample-5/`). See [the data flow](../../docs/data-flow.md) and [the ADRs](../../docs/adr/) for the overall picture.

Exports are cumulative, so one session shows up in many submissions. Submissions are folded newest first and only the newest copy of each session is used. All stored observations are folded in on every run; raw observations of fields with a custom merge are written to `output/.observations/` (removed after the run) and read back one part at a time, so memory use stays about flat.

The results are written to `output/`:

| File | Content |
|---|---|
| `foreverNpcTraces.lua` | NPC corrections |
| `foreverQuestTraces.lua` | Quest corrections |
| `foreverItemTraces.lua` | Item corrections |
| `foreverObjectTraces.lua` | Object corrections |
| `xpDB-traces.lua` | Quest XP rows (`QuestXP.db[questId] = {level, xp}`) for new quests, from `GetRewardXP`. Only samples where the quest is at most five levels below the player are used, as the client scales the reward down beyond that. The shown XP is rounded, so it is matched with the Forever `QuestXP` DBC table (`src/extract/resources/QuestXP.*.csv`) and the table's raw value is written; samples that match no value of the quest level are dropped. Place it in QuestieDB `support/Forever/QuestXP/` |
| `lookup{Npcs,Quests,Items,Objects}/<locale>.lua` | Translations, see below |
| `meta.json` | `sessionCount`, `submissionCount`, `exportCount`, `skippedFiles`, `partialFiles` (submissions of which only some exports could be decoded), `duplicateSessionsSkipped` and `generatedAt` of the run |

The trace-analyzer's Extract tab shows these files for copy/download.

### Translations

The same run writes Questie l10n lookup modules for every non-enUS locale found in the Forever traces, one directory per entity (`output/lookupNpcs/`, `output/lookupQuests/`, `output/lookupItems/`, `output/lookupObjects/`) with one file per locale (e.g. `output/lookupItems/deDE.lua`). Like the corrections, only IDs above the known Classic maximum are exported. NPC subnames are not traced and are always `nil`.

## How it works

1. `src/store/source.ts` lists the submissions and fetches new ones, `src/core/trace-data/` decodes them.
2. `foldSessions` (`src/extract/index.ts`) runs every observer in `src/extract/observers/` over the submission's sessions and collects their observations. Fields with the default scalar merge only keep a per-value summary; fields with a custom merge (spawns, drops, ...) keep every observation.
3. The observations of new submissions are added to the store (`src/store/observed-store.ts`), then every stored session is replayed in fold order.
4. After the last session, `finalize` aggregates the observations into one fact per entity and field, builds the per-entity records (`src/extract/emit/`) and writes the Lua modules (`src/extract/schema/corrections-writer.ts`).

### Quest-giver correlation

`startedBy` and `finishedBy` prefer a valid `questnpc` giver, using `npc` only as a fallback. NPC/object `questStarts` and `questEnds` are derived from those exported quest records so both sides agree.

Quest and `GOSSIP_SHOW` lookups include synchronous API samples recorded after the raw event, but stop before the next raw event in the same frame. Ambiguous clock ties are skipped rather than resolved to an arbitrary giver. Recorded nil transitions clear earlier GUIDs; unchanged GUIDs remain usable during long dialogs.

Gossip lists remain change-only. The extractor replays every `GOSSIP_SHOW` using the last observed available/active quest lists and the giver at that event. This recovers identical lists offered by different NPCs without changing recordings, including existing traces. An observed empty or nil list clears previous quests. No gossip relation is inferred without a `GOSSIP_SHOW` event.

As with GUID replay, missing samples retain the last observed state. The format cannot distinguish an unchanged successful read from a failed probe. A failed list read at a different NPC can therefore reuse an earlier list incorrectly; this is an explicit limitation of last-observed-state inference, not proof of a successful read at every event.

### Required races

`requiredRaces` hides quests from players, so it is only exported with proof that a whole faction cannot get the quest. Every starter must be an NPC that at least 3 sessions of that faction saw hostile and none saw friendly, while the other faction saw it friendly and never hostile. No session of the excluded faction may have seen the quest, nor another quest of the same starters (class quests have a counterpart quest giver in the other faction). The exported mask is Questie's Forever `ALL_ALLIANCE` / `ALL_HORDE`. Race-level restrictions are not inferred. NPC reactions are therefore collected for Classic NPCs too, but only exported for Forever NPCs.

## Development

```sh
npm test
npm run typecheck
```
