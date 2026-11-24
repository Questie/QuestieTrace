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

**Since (2026-11-23):** the archive grew to 199,967 submissions (998,061 episodes, 88,654
characters) and a run no longer fit next to anything else. Every step now reads the episodes one
bucket of whole characters at a time from an on-disk episode store, and absence runs on all cores:
14 min 28 s on 16 threads, peak 4.3 GB (ingest 657 s, signals 193 s, of which absence 52 s), with
the same values as before on the 5% sample.

The TypeScript side cannot run the full archive as it stands: the Exporters dump ran out of
JavaScript heap at 4,250 of 133,039 submissions (default 4 GB heap), and the relations signals
load every episode into memory (episodes.jsonl is 4.9 GB). A TypeScript full-archive reference was
not attempted (it would need well over 20 GB of heap on this 31 GB machine, with the memory errors
above).

## Review of the November 18 module (2026-11-21/22)

A QuestieDB maintainer reviewed the module QuestieDB had imported on November 18 (846 applied
relations on 643 quests) and reported three kinds of wrong output on Era quests:
- breadcrumbs overwritten: 1085 got preQuestSingle {1070}, 1799 its four breadcrumbs 4965, 4967,
  4968 and 4969, and 1842 got {1839, 1848} where Era had {1848};
- parent/child relations overwritten: the Chapter quests 339-342 (children of 338) got
  preQuestSingle {583};
- nextQuestInChain on a quest that opens several: 583 "Welcome to the Jungle" chained to 194.

Why it is an overwrite: QuestieDB loads Era's corrections, then this module, then the hand-written
`foreverQuestFixes`, so whatever the module writes for an Era quest replaces Era's value.

**What the hand-written fixes say.** The maintainer's commit `e96afe4` in QuestieDB ("Carry over
relations and redo overwritten corrections") copied 771 of the module's relations into
`foreverQuestFixes` and undid 24. "Copied" is not a review (they reviewed up to about quest 1085),
"undone" is. Of the 24, 20 linked two Era quests and 4 involved a Forever-new quest. In every
case but two, QuestieDB already held the facts that ruled the relation out:

| Pattern | Undone | Rule now in `combine/lua.rs` |
|---|---|---|
| A breadcrumb QuestieDB knows made a prerequisite of its target | 1085, 1799, 1842, 5244, 8234, 8414 | Never applied |
| A prerequisite implied by the parent quest or by an earlier step of a listed prerequisite | 339-342, 3450, 1122, 4001 | Never applied; only necessary links count (AND lists, one-quest preQuestSingle, QuestieDB's chains), not one option of an OR list |
| nextQuestInChain from a quest several quests require | 583, 376, 92514, 92679, 93461 (and 379, 92840 with two) | Not applied from a quest three or more quests require |
| nextQuestInChain into an AND list | 1434 | Already not applied (`never chain into an AND list`) |
| Not caught | 379, 92840 (two requirers, like 205 and 275, which were kept), 2300 (availableUntilCompleted), 1661 (a breadcrumb list that was right but incomplete) | Need a human |

Measured on the reviewed module: 13 of the 24 match the breadcrumb and implied rules (the module now
holds back 12 of them; 4001 is already in QuestieDB), and neither rule touches a relation that was
kept; the three-requirer chain rule catches 5 more and 8 relations beyond the reviewed range
(the same rule at two requirers would also drop 205 and 275, which were reviewed and kept). A
rejected alternative was to emit nothing between two Era quests: it removes 62 relations, 20 of
them undone but 30 kept (mostly exclusiveTo between Era quests), while new Forever quests are
often steps in old Era chains, so Era quests cannot simply be skipped. Rules hold relations back
as commented-out lines with the reason, so reviewers still see them.

**What it means now.** Since QuestieDB carries the 771 copied relations as hand-written fixes,
the full runs of November 23 apply 12 relations, or 14 against that day's QuestieDB master
(exclusiveTo pairs between same-name quests, a breadcrumb pair and new prerequisites); against
hand-written Forever relations the traces agree 2,594 times, add 28 and differ 16 times (listed in
`relations-review.md`; see "Next" for why those are not mistakes in the fixes). Improvements on those 771
only reach the game when someone acts on a conflict. Not done: parent/child relations are never
proposed (the model can represent parentQuest, but no signal looks for "only offered while quest
P is in the log").

To redo the check after another review: list the relation fields the hand-written fixes changed
since the module was imported (`git diff <import commit> -- src/corrections/Forever/foreverQuestFixes.lua`
in QuestieDB), and for each relation the module applies, whether the fixes kept it, undid it or
left it alone; the undone ones, next to what the ground truth (`groundtruth.json`) knew about
both quests, show the pattern.

## Why the traces get prerequisites wrong (2026-11-23/24)

State: QuestieDB PR #238 (branch `fix/relation-traces-review`) holds the module regenerated on
November 23 and waits for the maintainer's validation.

**Start from this principle: where the traces and a hand-written fix differ, assume the traces are
wrong until shown otherwise.** Of that run's 16 "conflict" rows (9 quests), none was shown to be a
mistake in the fixes.

Measured against the prerequisites in `foreverQuestFixes.lua` (QuestieDB `47a4756`: 1,495 on 798
quests). 499 of them were in our own module before they were in the fixes, so only the other 996 say
something. 499 of those were turned in before their quest by three or more characters; 380 more are
OR-list entries nobody turned in (mostly copies that complete together). Combined found 410 of the
499; of 794 claims on those quests 783 were right. What they got wrong has two causes:

1. **Recordings that are not from the live game** contradict correct prerequisites, and one
   contradiction is enough for the signals to drop one. "Rascally Rodents" (91743) got "Nibbled-On
   Book" (91741), one step too early, because one character on another server got it before "Book
   Inventory" (92124). Fixed in ingest, see "Beta and other servers". The guess that signals prefer
   a prerequisite turned in at the quest's own giver was wrong for it.
2. **Versions of a quest that complete together.** Turning in one flags all of them (Alliance and
   Horde, per race, per level bracket), so the traces claim versions the quest's characters never
   did: "The Fate of Zephras" (93836, 94369), "The Great Cat Spirit", "Call of Earth". The
   maintainer's answer (2026-11-24, checked in game on "Call of Earth": with 1516 done a troll is
   offered 1517 and, once that is abandoned, can take 1520): a prerequisite lists every version as
   preQuestSingle, and all versions of a step are exclusiveTo each other. Showing only the versions
   a player's faction has access to belongs in Questie's Journal, not in the data. So those claims
   are right, and the pipeline keeps them.

   Two rules were tried against them and not kept. Dropping claims no character turned in would drop
   the versions the maintainer wants listed. Dropping claims only the other faction takes, by which
   faction's characters accepted or turned the quests in, was built and taken out again: 212 of the
   224 accepted prerequisites it removed are in QuestieDB's own lists, all 224 are OR lists where the
   extra version changes nothing in game, and the verdict is wrong too often on few characters (seen
   in 5% and 20% of the characters and checked against all: 13% and 18% wrong with 3-4 takers, 3% and
   5% with 5-9, about 1% with 10-19, under 1% above).

   Not done: the "Call of Earth" values in QuestieDB itself (its steps 2 and 3 have no exclusiveTo,
   and 92468 requires only 92467). Measured with the agreed values put into the ground truth:
   `exclusiveTo` 1,548 -> 1,650 accepted, none conflicting, and 31 prerequisites differ from QuestieDB
   instead of 41. Until QuestieDB has them, their claims count as wrong in the model and hold other
   correct pairs back, so the module should not be regenerated for QuestieDB before that.

Found on the way, not done:
- **Race masks in QuestieDB are wrong on 87 quests** (three or more characters of an excluded race
  accepted the quest): 51 from the base data (Wowhead's Race and Side), 30 from the Classic data
  (no Skyborne and none of Forever's new class combinations), 4 hand-written, 2 guessed from the
  giver. So no rule may rest on the masks (the planned "never a prerequisite no character of the
  quest can take" is dropped), and signals that use them to decide who could take a quest are
  misled there. A task of its own; list in `relations-output-live/measure/race-masks.txt`.
- **Claims only cooccurrence makes** score 0.67 to 0.80 and are not accepted, also when thousands
  of characters back them. With 1,000 or more takers they were right 37 of 39 times. They are the
  largest group of misses with clean order (11 quests).
- **OR lists of real alternatives** (33 prerequisites on 11 quests) cannot be found: the signals
  look for one quest every taker had done.

Mistakes to avoid (made on 2026-11-23): reading only the `conflict` rows of `relations-review.md`
hides the `agree` rows of the same quest, which made correct fixes look swapped; "had it completed"
counted over a whole capture says nothing about order, so use the event order (`questEvents`,
`completed.changes` times) for anything about before and after; finding a field with grep over
QuestieDB's layers picks up the next quest's lines, so load the layers; and recompute every number
quoted to someone else from the data first (583 is required by four quests in QuestieDB, not eight:
the other four were this module's own wrong output).

How to look at a quest: `relations-review.md` section `### <quest id>` for the evidence,
`candidates/<signal>.json` for what each signal proposed, and a scan of `episodes.jsonl` for who
accepted, turned in or completed what. The runs and scripts are gitignored in
`tools/corrections-extractor-rs/`: `relations-output-questiedb/` (the November 23 run, with the beta;
`measure/` holds the scripts of this section, e.g. `versions.py` for a family of versions),
`relations-output-live/` (the same episodes without the beta and the other servers) and
`relations-output-clean/` (a full run with the filter).

## Beta and other servers (2026-11-23/24)

Ingest leaves out sessions that are not from the live game (`ingest/origin.rs`):
- **Beta sessions**, by GetServerTime: before 2026-11-04. The archive's last beta session is from
  October 23, the first one on the live realms from November 4, 23:05 UTC. 24,849 sessions of the
  November 23 archive's 998,061 (the full run with this filter keeps 973,119).
- **Sessions without a server time** (48), which tells the beta from the live game.
- **Characters met on a server that is not Blizzard's**, by the server id in their NPC GUIDs
  (`Creature-0-<server>-...`): 1 and 70, where Blizzard's had four digits. Six characters, 45
  sessions. Their sessions that met no NPC, and so do not show their server, go too; one on a
  server of Blizzard's stays, since grouping can put two characters together.

Why: a signal drops a prerequisite when one character lacked it, so a handful of such sessions
hides relations that thousands of characters confirm. Checked on 2026-11-23: of the hand-written
prerequisites with a small share of contradictions, all seven had none left without these sessions
(91743 "Rascally Rodents", 91724, 99129: one character on server 70; 92840, 96261, 95216, 92422:
beta only). A tolerance ("0.1% of the characters may contradict") was measured and not built: on
the clean data it would let 263 more pairs through, none of them a prerequisite QuestieDB has, and
it would hide the next case of bad recordings.

Every run logs what it left out, as a WARNING where somebody should look (another server, no server
time), and `reports/ingest.json` has the counts (`sessions.skipped`, `sessions.otherServers`).

What it changed on the full archive (accepted relations, before -> after): prerequisites 2,839 ->
2,905 (the same as QuestieDB 2,795 -> 2,839, different 27 -> 41), chains 1,456 -> 1,474, breadcrumbs
41 -> 42, `exclusiveTo` 1,602 -> 1,548. The last one is not evidence lost: 30 new claims between the
versions of "Call of Earth" (1516-1521, 92466-92468, 94373-94375, which complete together) count as
wrong because QuestieDB has no such relation, and refitting the model on them lowers the others just
below the acceptance score. Without those 30 claims it is 1,620, all 18 new ones as in QuestieDB.
The maintainer has since confirmed those pairs ("Versions of a quest that complete together" above).

The date and the server id only catch what is known today. For the rest, combine compares the
recordings with QuestieDB (`combine/contradicted.rs`): it lists every prerequisite QuestieDB has
that a few characters (at most 2%) could get or had the quest without, and which characters, under
"Recordings to check" in `relations-review.md`, and logs a WARNING with the count. Repeatable quests
and quests a signal sees as an optional lead-in are not checked. On the archive with the beta and
the other servers in it lists 15, naming five of the six characters; without them none.

## Reading the episodes one bucket at a time (2026-11-22/23)

The archive grew to 199,967 submissions and a full run no longer fit in memory next to anything
else (it was stopped at roughly 15 GB of an expected 24). Now no step holds every episode:

- `src/episode_store`: ingest writes the episodes in a compact binary format (a hand-written
  codec; every field listed once, so a new model field does not compile until it is encoded) in
  buckets of about 32 MB that end where a character ends, in ingest's order. 1.9 GB for the full
  archive against 7.3 GB of `episodes.jsonl` (still written, for people and the TypeScript tools).
- Ingest spills every session copy to 64 partitions by session key, merges them partition by
  partition, spills the kept sessions by contributor id, and builds characters one contributor
  partition at a time, in order (`ingest/spill.rs`). Character assignment only ever looks within a
  contributor, so the result is byte-identical.
- Every signal reads `Episodes::for_each_bucket`, in several passes where it needs them, and keeps
  only owned, compact state: characters as `CharacterId`s, ids or owned copies instead of
  references into a bucket. Absence also works per character on all cores within each bucket.
- After ingest and after each signal the process hands freed memory back (`malloc_trim`); before,
  its footprint only ever grew.

Checked on the 5% sample against the pre-change run, by value, with 1 MB buckets (68 boundaries)
and default ones, at 1 to 16 threads; each signal has a test that a bucket per character
(`PerCharacter`) finds what one bucket finds, and each such test was shown to fail on a
deliberately broken version. Full archive, 16 threads: 14 min 28 s, peak 4.3 GB (ingest 657 s and
2.2 GB, then each signal on its own: handoff 26 s, offer-set 22 s, absence 52 s instead of 368 s,
cooccurrence 60 s, breadcrumbs 34 s), with the same values as a 6-thread run.

What still grows with the archive: offer-set's per-quest folds (a few lists per view), absence's
blocked moments (formatted text per moment and suspect), cooccurrence's example evidence for every
surviving candidate rather than only emitted edges, and the store decodes each bucket on one
thread. Decoding the submissions is three quarters of a run.

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
- **absence** keeps evidence texts for every blocked moment; building them only for what it
  outputs would shrink its state (it now runs on all cores: 52 s on the full archive).
- **Using it from the dump**: the binary reads the same environment variables as the dump's
  `relations pipeline` step, so the dump could spawn it instead; Exporters was not touched.
- **Simplifying the extract port** the same way (value comparison, fewer JS-compatibility helpers)
  was the stretch goal and was not started.

## Where things are

- Branch `feat/relations-rs` (PR #137); the relations work is the `relations-rs` commits.
- The latest full run's deliverables: `tools/corrections-extractor-rs/relations-output-final/output/`
  (gitignored, November 23).
- The binary: `cargo run --release -p relations -- pipeline` (see the crate README for the
  environment). The whole-run comparison: `scripts/relations-compare.sh EXPECTED_DIR ACTUAL_DIR`.
- Scratch data (references, the scratch Exporters copy with the latest scraper data and QuestieDB,
  trace-data views, full-run outputs) is in
  `/tmp/claude-1000/-home-logon-projects-QuestieTrace/a3c07696-fc63-4b8f-b2c0-4546a87aa2e2/scratchpad/`:
  `ref5/` (TS reference), `rs-final-B/` (full archive, Rust), `exp/` (Exporters copy, ~13 GB of
  scraper databases). Safe to delete; the scratch directory may be gone after a reboot.
