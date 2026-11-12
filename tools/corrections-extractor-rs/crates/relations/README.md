# relations

Infers Questie quest relationship fields for WoW Forever (`preQuestSingle`, `preQuestGroup`,
`exclusiveTo`, `nextQuestInChain`, breadcrumbs, ...) from QuestieTrace recordings. It is a Rust
version of the Exporters pipeline in `tools/corrections-extractor/src/relations` (see the README
there for the reasoning behind each signal): the same inputs, steps, files and deliverables, with
outputs that match it by value.

A wrong relationship hides quests from players, and a wrong breadcrumb can make Questie
auto-abandon a quest, so precision beats recall. The output is a reviewable candidate set, never an
automatic import.

## Running it

```sh
cd tools/corrections-extractor-rs
export TRACE_DATA_DIR=...                # trace-data checkout (submissions/<yyyy-mm>/*.json)
export QUESTIEDB_DIR=...                 # QuestieDB checkout (its Lua loader builds the catalog)
export SCRAPER_DIR=.../v2026.11.10       # scraper release folder (data/raw, data/parsed)
export RELATIONS_TRACE_MODULES_DIR=...   # optional: a dump's own forever*Traces.lua
export RELATIONS_REQUIRE_FOREVER_VERSION=1
export RELATIONS_DIR=.relations          # working files (default .relations)
cargo run --release -p relations -- pipeline
```

The environment variables are the TypeScript pipeline's, so the binary is a drop-in for the
Exporters dump's `relations pipeline` step. Deliverables go to `$RELATIONS_OUTPUT_DIR` (default
`$RELATIONS_DIR/output`): `foreverQuestRelationTraces.lua`, `relations-review.md` and
`relations-meta.json`. Read the review before trusting the module.

The catalog runs QuestieDB's own Lua loader (QuestieDB's bundled Lua 5.1 on x64 Linux/Windows,
else `lua5.1` on `PATH`), exactly as the TypeScript pipeline does.

Every step is also a command of its own that reads what the previous steps wrote:

| Command | Reads | Writes (in `RELATIONS_DIR`) |
|---|---|---|
| `ingest` | trace-data | `episodes.jsonl`, `reports/ingest.json` |
| `catalog` | QuestieDB (+ trace modules) | `catalog.json`, `groundtruth.json`, `reports/catalog.json` |
| `source:wowhead` | scraper databases | `candidates/wowhead-*.json`, `reports/wowhead-inventory.json` |
| `signal:<name>` | episodes, catalog | `candidates/<name>.json`, `reports/<name>-*.json` |
| `combine` | all candidates, catalog, ground truth, episodes | `relations.json`, `reports/combine-metrics.json`, the deliverables |
| `score <signal> [--min-score N]` | one candidates file, ground truth | `reports/score-<signal>.json` and a table |

`pipeline` runs ingest, catalog, the Wowhead source, every signal (one after another, each on all
cores) and combine in one process, handing each step's output straight to the next and writing every
file on the way. Combine gets exactly the candidate files the run produced; the `combine` command
reads whatever `candidates/` holds. There is no incremental cache: a full run decodes every
submission, which is what keeps it simple and always consistent.

## How it is built

```
trace-data --ingest--> episodes --+
QuestieDB --catalog--> catalog ---+--signals--> candidates --+
scraper --wowhead--> context candidates ----------------------+--combine--> Lua, review, meta
```

- `model.rs` holds every contract between the steps: episodes, the catalog, the ground truth and
  candidates, with the TypeScript files' field names (each side reads the other's files).
- Each step is a module whose `run` returns a typed output that knows how to `write` itself; the
  pipeline in `main.rs` only sequences them.
- Shared helpers: `timeline.rs` (completed/in-log/level at a time, per episode), `eligibility.rs`
  (race/class/level gating), `race_masks.rs`, `score.rs` (precision/recall against the ground
  truth), `text.rs` (numbers in text as the TypeScript pipeline prints them), `io.rs`.
- `ingest` reads submissions with the `trace-data` crate on all cores (one arena per worker, reset
  after each submission), distills each session to an episode, then merges the copies of a session,
  resolves legacy greeting titles and groups sessions into characters on one thread.
- Signals never read the ground truth; combine uses it to calibrate a logistic model per relation
  kind and to measure itself. Wowhead claims are context for reviewers only: they never decide.

### Adding a signal

1. Write `src/signals/<name>/mod.rs` with a type implementing `signals::Signal`:
   `name()` (the candidates file name and the `signal:<name>` command), `describe()`, and
   `run(&SignalInput) -> Findings` (its candidates, covered quests, input count, params and any
   reports; the pipeline adds the signal name and `generatedAt` to make the candidates file).
   Keep its tunables in one `#[derive(Serialize)]` struct and write that as `params`, so none
   can go missing from the file.
2. Add it to `signals::registry()`.

The pipeline then runs it with the others, writes its files, `score <name>` measures it, and
combine picks its candidates up as one more model column. Fill `covered_quest_ids` honestly
(every quest the signal could judge, with or without a candidate), or recall means nothing.

Two places outside the module go by signal name and may need a line:

- `combine/assumptions.rs` (`phrase`): how the Lua module's `-- Assumption:` comment words the
  signal's evidence, per field. Without an entry the comment just says `<name> signal`.
- `combine/inputs.rs` (`is_context_only`): files named `wowhead` or `wowhead-...` are context for
  reviewers only and never decide anything. A new source like that needs a name it matches; a new
  signal must not match it.

Rules every signal follows (they are why the output can be trusted): never read the ground truth;
gate negative evidence ("X was not offered" only counts when the character could have taken X:
level, race, class, not completed, not in the log, and the giver starts X); count independent
characters, not sessions; keep a few concrete evidence lines per candidate; only Forever sessions
(`signals::forever_episodes`); no player names, GUIDs, realms or guilds anywhere.

## Checking it against the TypeScript pipeline

`examples/compare.rs` compares two files by value (JSON key order ignored, numbers within 1e-9,
`generatedAt` and timestamps ignored, JSONL records matched by `key`):

```sh
cargo run --release -p relations --example compare -- ts/relations.json rust/relations.json
scripts/relations-compare.sh TS_RELATIONS_DIR RUST_RELATIONS_DIR   # every file of a run
```

## Development

```sh
cargo test -p relations
cargo test -p relations -- --ignored   # also the catalog tests that need QUESTIEDB_DIR and Lua
cargo clippy --all-targets -- -D warnings
```
