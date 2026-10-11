# Quest relationship inference

Infers Questie quest relationship fields (`preQuestSingle`, `preQuestGroup`, `exclusiveTo`,
`nextQuestInChain`, breadcrumbs, ...) for WoW Forever from QuestieTrace gameplay traces. Wowhead
is shown to reviewers as context only and never decides anything (see below). Field semantics are documented on `RELATION_FIELDS` in
`core/types.ts`; they mirror Questie's `QuestieDB.IsDoable`.

The goal is reviewable, evidence-backed candidates first; writing Questie corrections comes last.
A wrong relationship hides quests from players, and a wrong breadcrumb can make Questie
auto-abandon a quest, so precision beats recall.

## Why this can work

- Servers leave unavailable quests out of a giver's list entirely (they are not shown grayed
  out), so a complete offer list tells us what was *not* available, not only what was.
- Traces record the character's full completed-quest history at capture start plus every change,
  the quest log over time, and every giver offer list with quest ids and the giver's id.
- 7,000+ submissions from about 1,900 characters give repeated, independent observations per quest.

## Data flow

```
trace-data/submissions/**.json --ingest-->  .relations/episodes.jsonl        (QuestEpisode per line)
QuestieDB data + corrections   --catalog--> .relations/catalog.json           (QuestCatalog)
                                            .relations/groundtruth.json       (GroundTruth, scoring only)
scraper-questie forever.db     --source:wowhead--> .relations/candidates/wowhead-*.json
episodes + catalog             --signal:*-->       .relations/candidates/<signal>.json
all candidates                 --combine-->        output/foreverQuestRelationTraces.lua   (deliverable)
                                                   output/relations-review.md              (human review)
                                                   output/relations-meta.json              (run summary)
                                                   .relations/relations.json, .relations/reports/combine-metrics.json
```

`.relations/` lives in `tools/corrections-extractor/` and is gitignored. Override locations with
`RELATIONS_DIR`, `TRACE_DATA_DIR`, `QUESTIEDB_DIR`, `SCRAPER_DIR` and `RELATIONS_OUTPUT_DIR` (see
`core/paths.ts`).

## Deliverables

`combine` publishes into `tools/corrections-extractor/output/` (gitignored), next to the four
`forever*Traces.lua` modules from `npm run extract`:

- `foreverQuestRelationTraces.lua`: module `ForeverQuestRelationTraces`, relationship fields only, so
  it never overlaps the field extractor's modules.
- `relations-review.md`: read this before trusting the module.
- `relations-meta.json`: generatedAt, min score, episode and character counts, quests and relations in
  the Lua, authored agree/new/conflict counts, inputs used, and the metrics leakage caveat. It has its
  own name because `npm run extract` rewrites `meta.json` and its four modules (and nothing else) in
  the same folder. Combine removes it first and writes it last, so it never describes a half-written run.

Setting `RELATIONS_DIR` (a scratch run) moves the deliverables to `$RELATIONS_DIR/output/` as well, so
scratch runs and tests never overwrite the real ones; `RELATIONS_OUTPUT_DIR` sets the folder explicitly.

The trace analyzer (`tools/trace-analyzer`, `npm run dev`) shows them in its Extract view, "Quest
relations" tab: the module for copy/download with the run summary, and the review report. It always
reads `tools/corrections-extractor/output/` and ignores `RELATIONS_DIR` and `RELATIONS_OUTPUT_DIR`, so
scratch runs never show up there.

**Shipping is manual, in QuestieDB (a separate repo):** copy `foreverQuestRelationTraces.lua` into
`src/corrections/Forever/traces/`, and register `ForeverQuestRelationTraces` in `src/config.lua` next to
the `ForeverQuestTraces` entry (`datatype = 'Quest'`, `static = {'Load'}`, `generated = true`,
`window = 'Forever'`), so it loads after the generated base and before the authored
`foreverQuestFixes.lua`, which overrides it per field.

## Commands

```sh
cd tools/corrections-extractor
npm run relations                        # list commands
npm run relations -- ingest              # build/refresh episodes.jsonl (incremental)
npm run relations -- catalog             # build catalog.json + groundtruth.json (needs Lua 5.1; uses QuestieDB's own loader)
npm run relations -- signal:handoff      # any signal or source writes candidates/<name>.json
npm run relations -- score handoff       # precision/recall of candidates/handoff.json vs ground truth
npm run relations -- combine             # all candidates -> deliverables in output/ (see Deliverables)
npm run relations -- pipeline            # every step in order, ingest through combine
```

## Areas and ownership

Each area is a folder with a `run.ts` exporting `run(args)`, wired up in `cli.ts`. An area only
writes inside its own folder and its own cache outputs.

| Area | Folder | Reads | Writes |
|---|---|---|---|
| Contracts and shared helpers | `core/` | - | - |
| Ingest | `ingest/` | trace-data | `episodes.jsonl`, `ingest-cache/` |
| Catalog and ground truth | `catalog/` | QuestieDB | `catalog.json`, `groundtruth.json` |
| Wowhead relations | `sources/wowhead/` | scraper-questie DBs | `candidates/wowhead-*.json` |
| Hand-off chains | `signals/handoff/` | episodes, catalog | `candidates/handoff.json` |
| Offer-time completed sets | `signals/offer-set/` | episodes, catalog | `candidates/offer-set.json` |
| Absence from complete offer lists | `signals/absence/` | episodes, catalog | `candidates/absence.json` |
| Completion-history co-occurrence | `signals/cooccurrence/` | episodes, catalog | `candidates/cooccurrence.json` |
| Breadcrumbs | `signals/breadcrumbs/` | episodes, catalog | `candidates/breadcrumbs.json` |
| Combination and output | `combine/` | all candidates, ground truth, episodes (counts only) | `relations.json`, `reports/combine-metrics.json`; deliverables in the tool's `output/` (see Deliverables) |

Ingest reads and decodes submissions through `src/core/trace-data/`, the same reader the field
extractor's `--trace-data` mode uses, so both pipelines see identical sessions.

`sources/wowhead` writes one candidate file per page surface: `wowhead-series`, `-storyline`,
`-requires`, `-requires-any`, `-requires-in-progress`, `-unlocks` and `-disables`. Forever pages only
carry Series and Storyline; the other panels exist only on Era pages the scrape inherited, so a
missing panel on a Forever page says nothing. Wowhead has no breadcrumb surface. Each candidate's
`note` records surface, shape and provenance (`forever-new-edge`, `forever-era-edge`, `era-page`).

**Wowhead is untrusted.** Series relationships are unreliable even when they look unambiguous, so
combine keeps Wowhead out of the model entirely: it only appears in the review report as labelled
context. Its scores against `groundtruth.json` are also circular: Questie's existing data was partly
scraped from the same Series rows, so high agreement there says little about Forever truth.

`core/` holds the contracts (`types.ts`), cache paths and I/O (`paths.ts`, `io.ts`), point-in-time
lookups over an episode (`timeline.ts`), race/class/level gating (`eligibility.ts`), scoring
(`score.ts`) and test builders (`fixtures.ts`).
Treat `core/` as read-only from inside an area; propose changes instead of making them.

## Rules for every area

- **Privacy.** Episodes, candidates and reports never contain player names, player GUIDs, realms,
  guilds or free text from chat/gossip. Contributor and character keys are pseudonymous. Quest and
  NPC names are fine.
- **Signals never read ground truth.** `groundtruth.json` is for scoring and combination only.
  The catalog is fine to read (levels, races, classes, starters) for gating evidence.
- **Gate negative evidence.** "Quest X was not offered" only counts when the character could have
  taken X otherwise: right level, race, class, skill, not already completed, not in the log, and
  the giver actually starts X. Prefer evidence that holds across several characters.
- **Coverage is honest.** `coveredQuestIds` lists every quest the signal had enough data to judge,
  including ones where it found no relationship, so recall is meaningful.
- **Forever only by default.** Use episodes with an interface version starting with 16, plus ones
  without a version. Old Classic traces may be used for validation, never mixed in silently.
- **Evidence is reviewable.** Each candidate carries a few concrete examples a person can check.
- **No new npm dependencies.** Node built-ins (`zlib`, `node:sqlite`, `readline`) and what
  `package.json` already has.
- Code style follows the rest of `tools/corrections-extractor`: strict TypeScript, small modules,
  focused vitest tests next to the code (`*.test.ts`), comments that explain why.
