# Quest relations in Rust: overnight report (2026-11-12)

A natural Rust version of the Exporters quest-relations pipeline (`src/relations` in Exporters):
`tools/corrections-extractor-rs/crates/relations`, on the local branch `feat/relations-rs`
(not pushed). Same inputs, steps, files and deliverables; outputs compared by value with the
TypeScript pipeline. Crate docs, how to run it and how to add a signal:
[crates/relations/README.md](tools/corrections-extractor-rs/crates/relations/README.md).

## First: this machine has memory errors

Several runs tonight crashed or produced corrupted data. The decisive evidence: two runs of the
same build wrote an `episodes.jsonl` differing in exactly one byte, `0x32` ('2') became `0xB2`
inside a JSON number (bit 7 flipped; no code path can write that byte there). Every other symptom
fits the same cause: segfaults (general protection faults on non-canonical pointers, i.e. a flipped
high pointer bit) in safe Rust, a U+FFFD in a pure-ASCII export payload (an ASCII byte with bit 7
set), "truncated CBOR" on bytes that had just validated, and a full-archive run that lost two
submissions (run A below). They showed up under heavy load (several builds and runs at once) and
with both mimalloc and the system allocator; no rerun reproduced a given one.

So results produced on this machine can carry silent bit flips, including other projects' builds
and data. Worth a memory test (Windows Memory Diagnostic or MemTest86) and a look at the WSL
memory settings. Tonight's results were guarded against it (see "How the results were checked").

## Results

**5% sample** (6,760 submissions, 35,864 episodes), idle machine:

| Step | TypeScript | Rust |
|---|---|---|
| ingest | 107.6 s | 17.5 s |
| catalog (QuestieDB's Lua loader) | 1.1 s | 1.1 s |
| source:wowhead | 8.5 s | 0.5 s |
| signal:handoff | 6.8 s | 0.4 s |
| signal:offer-set | 18.9 s | 0.6 s |
| signal:absence | 37.9 s | 7.1 s |
| signal:cooccurrence | 104.5 s | 2.1 s |
| signal:breadcrumbs | 7.5 s | 0.5 s |
| combine | 2.5 s | 0.4 s |
| **pipeline** | **296 s, 4.0 GB** | **30.6 s, 2.7 GB** |

**Full archive** (133,039 submissions, 639,682 episodes, 64,702 characters): Rust 11 min 6 s, peak
15.3 GB (ingest 364 s, signals 288 s, of which absence 171 s and cooccurrence 79 s). The deliverable:
627 quests, 1,646 relations; for Forever-new quests without authored relations, 394 quests / 820
relations; against authored Forever relations 1,555 agree, 14 new, 11 conflict.

The TypeScript side cannot run the full archive as it stands: the Exporters dump ran out of
JavaScript heap at 4,250 of 133,039 submissions (default 4 GB heap), and the relations signals
load every episode into memory (episodes.jsonl is 4.9 GB). A TypeScript full-archive reference was
not attempted (it would need well over 20 GB of heap on this 31 GB machine, with the memory errors
above).

## Parity with the TypeScript pipeline

Reference: the TypeScript pipeline (Exporters HEAD 9347a87) on the 5% sample, run exactly as the
dump runs it (`relations pipeline` with the dump's environment), with the latest scraper release
`v2026.11.10`, QuestieDB master `c7ccd23` and the field modules of a dump of the same sample.

- **Every file, end to end from raw submissions, has the same values**: episodes.jsonl, catalog,
  ground truth, every candidates file (5 signals, 7 Wowhead surfaces), every report, relations.json,
  combine-metrics, and the three deliverables. The Lua module and the review are identical line for
  line apart from the generation time. "Same values": JSON compared as data (key order ignored,
  numbers within 1e-9). Even at tolerance 0 everything matches exactly except four fitted model
  coefficients in relations.json, which differ around the 15th significant digit (the logistic
  fit's floating-point path); no probability, decision or printed number changes.
- **One input differs on purpose: 19 submissions** that Exporters' older export reader rejects
  ("payload is not valid print-encoded data (character "!")"). Each holds a cut-off export with a
  complete export pasted after it; the old regex swallowed the second start marker. The Rust reader
  (the newer QuestieTrace decoder) drops the cut-off export and reads the complete one. With them:
  +158 sessions; the deliverable's relation values are identical, only evidence counts and scores in
  comments move. The parity runs above use the view without those 19 files, where both agree.
- `reports/ingest.json` differs only in bookkeeping: run time, the TS cache version (there is no
  cache in Rust), the 19 files.
- Text matches TypeScript's number printing where it shows (`toFixed` rounds exact ties away from
  zero, e.g. 13/16 prints 0.813), through one helper (`crate::text`); everything else is plain Rust.

## How it is built

`model.rs` holds every contract (episodes, catalog, ground truth, candidates) with the TypeScript
field names, so either side reads the other's files. Each step is a module whose `run` returns a
typed output that writes itself; `pipeline` sequences them in one process and writes every file on
the way; every step is also a command on its own. Signals implement a small trait (`name`,
`describe`, `run(&SignalInput) -> Findings`) and are listed in one registry: adding a signal is one
module and one registry line, and combine picks it up as one more model column. No per-submission
cache: a full run decodes everything (ingest reads on all cores with one arena per worker).

## How the results were checked

- Every area was ported by its own agent and gated against the TypeScript reference before merge;
  the integrated branch was gated end to end after every merge and refactor.
- Two fresh reviewers (architecture/expandability, correctness/tests) reviewed the integrated crate;
  their findings were fixed (signals one at a time, shared helpers, robust ingest, a NaN-safe sort,
  score validation, exact float read-back, private internals), each followed by the gate.
- Tests: 259 relations tests (4 more need a QuestieDB checkout and Lua: they pass against the
  pinned master), plus the corrections (433) and trace-data (43) suites. Agents mutation-checked
  their tests (deliberately broken code must fail a test).
- Against the memory errors: a second idle TypeScript run matched the reference, so the reference
  is clean; the full-archive Rust run B's 4.9 GB episodes.jsonl is byte-identical to two earlier,
  independent full ingests (one with mimalloc, one with the system allocator). Run A of the same
  binary lost two submissions to bit flips and was discarded. Run B's signals and combine ran once:
  a confirmation run, after checking the machine's memory, is the one thing left to do.

## Found along the way (Exporters, not changed)

- The Exporters dump cannot process today's archive: it ran out of heap at about 3% of it.
- Exporters' export reader rejects submissions with a pasted export (19 on the 5% sample).
- The relations catalog needs QuestieDB master's layout: the restructured `forever` branch (your
  `~/projects/QuestieDB` checkout) has no generated base and would silently lose nearly every
  Forever-new quest. The dump refuses it; the Rust catalog now refuses it too.

## Not done, and follow-ups

- **Confirmation run** of the full archive after the memory check (Claude Code stopped a job of
  mine tonight for low memory, so I left it).
- **Catalog strictness**: values of an unexpected type in QuestieDB's Lua output are skipped (as in
  TypeScript); counting and failing on them would catch a layout drift.
- **Deliverable wording** still names the TypeScript commands (`npm run relations -- combine`),
  kept for line-for-line parity; reword when the Rust binary is what ships.
- **absence** is the slowest step at full scale (171 s); its second pass builds evidence texts it
  could build only for what it outputs.
- **Using it from the dump**: the binary reads the same environment variables as the dump's
  `relations pipeline` step, so the dump could spawn it instead; Exporters was not touched.
- **Simplifying the extract port** the same way (value comparison, fewer JS-compatibility helpers)
  was the stretch goal and was not started.

## Where things are

- Branch `feat/relations-rs`, starting at `f1ab49b` (skeleton) on top of the extractor work; about
  40 commits, one per area, fix and refactor. `git log f1ab49b..feat/relations-rs`.
- The binary: `cargo run --release -p relations -- pipeline` (see the crate README for the
  environment). The whole-run comparison: `scripts/relations-compare.sh EXPECTED_DIR ACTUAL_DIR`.
- Scratch data (references, the scratch Exporters copy with the latest scraper data and QuestieDB,
  trace-data views, full-run outputs) is in
  `/tmp/claude-1000/-home-logon-projects-QuestieTrace/a3c07696-fc63-4b8f-b2c0-4546a87aa2e2/scratchpad/`:
  `ref5/` (TS reference), `rs-final-B/` (full archive, Rust), `exp/` (Exporters copy, ~13 GB of
  scraper databases). Safe to delete.
