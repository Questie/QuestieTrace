# corrections-extractor-rs: plan

**How to use this file.** The first part, "Next: the Rust extractor becomes the primary
extractor", is the work to do, in order. Everything under "History" further down is done: it
explains how the Rust tools got their speed (2026-11-10/11) and must not be redone. The relations
port is done too ([RELATIONS.md](RELATIONS.md), PR #137).

# Next: the Rust extractor becomes the primary extractor

**Decision (2026-11-12):** the Rust tools in `tools/corrections-extractor-rs` become the
primary way to turn trace-data into corrections; the TypeScript extractor
(`tools/corrections-extractor`) is retired once the Rust one covers everything it is used for.

Relations was already written as natural Rust (outputs compared by value). The field extractor
(`crates/corrections`) was not: it was ported to be byte-identical with the TypeScript tool, down
to error texts, so it carries a layer that imitates JavaScript. As the primary extractor it no
longer needs to: the TypeScript tool stops being the definition of right, and becomes a reference
for one last comparison.

## Context for a fresh session

- **Code:** `tools/corrections-extractor-rs`, a Cargo workspace with three crates: `trace-data`
  (reads submissions and decodes them; shared, fast, keep as it is), `corrections` (the field
  extractor, this plan's subject) and `relations` (done, natural Rust; a model for the target
  style). The workspace's `README.md` and `DESIGN.md` explain the extractor (what must match the
  TypeScript tool, and why), `crates/relations/README.md` the relations pipeline.
- **Branch and PR:** all of this work goes on `feat/relations-rs`, PR #137 to master: the Rust
  conversion branch. Commit there, one commit per change. The owner works in this checkout:
  never rebase or force-push; bring master in with a merge commit, and ask before that too.
- **Data:** `~/projects/trace-data` (about 140,000 submissions, not a git checkout, synced from the
  bucket). Both extractors take the same 5% sample (SHA-256 of the submission key), so samples
  are comparable; develop on 5% (about 25 s in Rust), run the full archive at milestones.
- **Commands:**
  - Rust checks: `cd tools/corrections-extractor-rs && cargo fmt --check && cargo clippy
    --all-targets -- -D warnings && cargo test`.
  - Rust extractor: `cargo run --release -p corrections -- --trace-data ~/projects/trace-data
    --sample 5 --output <dir>` (full archive: drop `--sample`; about 7 min, 11 GB).
  - TypeScript extractor (the reference): run a copy of master's
    `tools/corrections-extractor` (`git archive origin/master tools/corrections-extractor`,
    `node_modules` symlinked from the checkout) with `TRACE_DATA_DIR=~/projects/trace-data
    EXTRACT_SAMPLE=5 TRACE_STORE_DIR=<a scratch dir> npm run extract`. Always set
    `TRACE_STORE_DIR` to a scratch directory: its default, `../trace-session-store` next to the
    checkout, is the real store. A full archive run takes about 100 min and 19 GB.
  - Comparing: `scripts/parity.sh` (extractor, byte comparison today; step 0 changes it),
    `scripts/relations-compare.sh` and `crates/relations/examples/compare.rs` (value comparison).
- **This machine has memory errors** (RELATIONS.md): a run can crash or carry a flipped bit. Run
  full-archive jobs one at a time, do them twice and compare when the result matters, and do not
  hold several GB in comparison tools (Claude Code stops background jobs under memory pressure).
- **Working style that worked:** a gate after every change, one commit per change, subagents in
  worktrees for independent parts with a shared brief, fresh reviewer subagents before calling a
  step done (see CLAUDE.md).

## Where we start

- **Rust extractor:** byte-identical to the TypeScript extractor as of #129 on the full archive,
  7 min 22 s against 101 min. Behind master by three changes (below).
- **Rust relations:** natural Rust, same values as Exporters' TypeScript pipeline on a 5% sample;
  11 min on the full archive. `scripts/relations.sh` checks inputs, fetches the scraper databases
  into the gitignored `data/scraper/` and runs it.
- **What the TypeScript extractor does that the Rust one does not** (from `docs/data-flow.md`):
  - reads new submissions straight from the OVH bucket (rclone, batches of 500); Rust reads a
    local trace-data checkout;
  - keeps every observation in a store (`../trace-session-store`: Parquet plus `views.sql`) that
    is queried with DuckDB for audits, e.g. `WHERE token = 'npc'` (ADR 0005); Rust keeps none;
  - observes only new submissions (the store); Rust re-reads everything, which takes 7 minutes.
- **The JavaScript-imitating layer in `crates/corrections`** (12.4k lines in all):
  - `js.rs` (196 lines): `js::number` (JS `Number()` on strings, 16 call sites), `js::trim` and
    `js::is_whitespace` (11), `js::round` (`Math.round`, 6), `js::to_fixed_4` (spawn dedupe keys,
    2), V8's `hypot` (1), and `min`, `max`, `sign`, `string`, `format_number`, `is_integer`;
  - the `Js` value model the Lua writer builds output from (`lua.rs`, 74 uses);
  - error-message parity in `trace-data`: zlib's error texts through the C zlib (`flate2`), the
    only reason that dependency exists; UTF-16 quoting of bad characters in print decoding.
- **Not JavaScript imitation, and staying:** `trace-data`'s table-shape rules (when a Lua table is
  an array, key order). They are how the data is read by both pipelines, and relations' parity
  rests on them. The arena, lazy decoding and the other speed work stay as they are.

## What "done" means

1. The Rust extractor produces everything the team uses from the TypeScript one, from current
   master's behaviour: the `forever*Traces.lua` modules, `xpDB-traces.lua`, the lookups and
   `meta.json`.
2. No JavaScript imitation is left except where an output depends on it, each such place named in
   a comment (as relations does with `crate::text`).
3. The audit workflow survives in some form (decision 2 below).
4. CI builds, lints and tests the Rust workspace; the docs describe the Rust tools as the way to
   run things; an ADR records the switch.
5. The TypeScript extractor is deleted, or explicitly kept as a frozen reference.

## Work, in order

Every step ends with the gate and a commit of its own. An output change is allowed only when it
is understood: the commit message says what changed and why, with counts.

Get answers to the four open decisions below before step 3; steps 0 to 2 do not depend on them.

**0. Reference and gate (small).**
- Bring master in first: merge `origin/master` into the branch (a merge commit, after asking the
  owner). The branch's copy of `tools/corrections-extractor` is from before #131; after the merge
  it is master's, and `scripts/parity.sh`, which archives `HEAD`, runs the right TypeScript.
- Freeze a reference: the TypeScript extractor's output on the 5% sample (and the full archive,
  about 100 min, on an idle machine), with the commit recorded next to it.
- Move `scripts/parity.sh` from byte comparison to value comparison: Lua modules line by line
  with timestamps masked, JSON by value (the relations compare tool already does both), and
  `meta.json`'s skipped and partial files compared by file name and count, not by reason text.

**1. Catch up with master, written as plain Rust from the start (small).** Do this before
anything else changes output: until then the Rust extractor knowingly differs from master's
TypeScript in quest XP and quest finishers, and every later comparison would show these first.
- #131: `questXp` is kept for every quest seen in Forever, not only Forever-new ids.
- `8508e3b`: `finishedBy` gets the token rules `startedBy` already has (and the Rust port already
  follows): `questnpc` at `QUEST_COMPLETE` and `QUEST_TURNED_IN`, `npc` only as the fallback for
  gossip quest lists, objects only from `questnpc`.
- `62cad93` (ADR 0005): `startedBy` and `finishedBy` observations record the unit token they came
  from. TypeScript stores it in a store column; in Rust it feeds the audit output of step 3.
- Gate: the same values as the step-0 reference.

**2. Remove the JavaScript layer (medium), one item per commit, most valuable first.**
- Replace the `Js` model in `lua.rs` with typed records written straight to Lua.
- `js::number` becomes explicit parsing of what the data holds (digits, a sign, a decimal point)
  with the cases that differ handled on purpose.
- `js::round` becomes `f64::round`: identical for positive values, and coordinates are positive.
- `js::to_fixed_4` spawn keys become integer keys (coordinates in ten-thousandths).
- V8 `hypot` becomes `f64::hypot` (may move a last bit; the gate shows whether any decision flips).
- Error texts in plain Rust words; drop `flate2` and the UTF-16 error path from `trace-data`.
- `js::trim`: one JS-whitespace helper shared with relations (`crate::text` today) only where
  names are compared; plain `trim` elsewhere.
- Shared helpers between `corrections` and `relations` get one home only when they are the same
  responsibility (AHA): number text for the TypeScript-compatible outputs, the race/class masks.
- Gate after each item: the same values as the step-1 output, or an understood difference.

**3. Daily-use parity (decisions first, then medium).**
- Source: keep "sync the bucket, then run" (decision 1).
- Audits: write the observations the run made, with their token, as a file DuckDB reads
  (decision 2), and port the useful `views.sql` queries.
- Incremental runs: not needed at 7 minutes per full run; revisit when a full run passes ~30
  minutes.

**4. Switch over (small).**
- CI: a Rust job next to the TypeScript one (`cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test`); the TypeScript job goes with the TypeScript tool.
- Docs: `docs/data-flow.md`, `README.md`, `AGENTS.md` and the tool READMEs; ADR 0006 "The Rust
  extractor is the primary extractor" (superseding the store parts of ADRs 0001 and 0003).
- One full-archive run, twice, compared (this dev machine has memory errors, see RELATIONS.md),
  checked against the step-0 full-archive reference.
- Retire `tools/corrections-extractor` (decision 3).

## Open decisions

1. **Reading the bucket:** keep "rclone sync, then run on the local checkout" (simplest, works
   today), or read the OVH bucket natively from Rust? Recommendation: keep the sync.
2. **Audits without the TypeScript store:** (a) write observations as JSON lines, which DuckDB
   reads directly (`read_json`), one file per run, no new dependencies; (b) write Parquet like the
   TypeScript store (the `arrow`/`parquet` crates are large); (c) no audit output. Recommendation:
   (a).
3. **The TypeScript extractor:** delete it at the switch, or keep it frozen as a reference for one
   release cycle. Recommendation: keep it frozen until the first release built from the Rust
   output, then delete.
4. **One home:** Exporters has its own, older copy of the TypeScript extractor and its dump spawns
   the TypeScript relations pipeline. Point the dump at the Rust binaries (they read the same
   environment variables), or move the Rust tools to Exporters? Either way, only one copy of the
   extraction rules should stay alive.

## Risks

- **Silent output changes** hidden by the value comparison: every allowed difference is listed in
  its commit message with counts, and the step-0 reference stays as the baseline until the switch.
- **Losing the audit habit:** decision 2 before step 4, not after.
- **The dev machine's memory errors** can corrupt any run: full runs are done twice and compared.
- **Exporters drifting further** while this happens: decision 4 early.

## Size

Steps 0, 1 and 4 are small (a day together); step 2 is the bulk (about the size of one relations
area per item group, gated each time); step 3 depends on decision 2 ((a) is small). Smaller than
the relations port: the observers' logic stays, only the layer around it changes.

# History: optimization night (2026-11-10/11), done

**Done, not tasks.** This is the log of how the Rust extractor got from 18 min 43 s to 7 min 22 s
on the full archive; every change it lists is already in the code. Read it to know why the code
is shaped as it is (arena, lazy events, trusted CBOR walks, ...), not as work to do.

Overnight work on the Rust prototype in `tools/corrections-extractor-rs` (local only, never pushed).
Goal: find out which optimizations pay off, by how much, and at what complexity; then measure the
full archive with both tools.

## Decisions (2026-11-10)

- **Speed is the priority.** A memory saving is worth it when it costs at most ~1% speed per 10%
  memory saved.
- **One local commit per kept experiment**; find the best combination.
- **Full-archive Rust runs** at milestones, so we know how the variants behave at real scale (the
  5% sample does not show finalize and fold costs that grow with the archive).
- **The full TypeScript run comes last**, once the Rust work is done.

## Rules for every experiment

- **Measure A/B, back to back:** the previous best binary against the candidate, alternating, two
  runs each, on the 5% sample (6,760 submissions), on an idle machine. Record elapsed, user and
  sys time and peak memory (`/usr/bin/time`). One run is noise; a change counts only when both
  candidate runs beat both baseline runs or the difference clearly exceeds the spread.
- **Parity gate:** `scripts/parity.sh 5` against the TypeScript reference output must stay
  identical, and `cargo test` and `cargo clippy --all-targets` must stay clean.
- **Keep or drop:** a change that helps is committed on its own (one commit per experiment);
  one that does not is reverted and recorded below with its numbers, so nobody tries it again
  blindly.
- **Re-profile** (samply) after every few accepted changes, so the order follows the data.
- **Complexity** is rated low / medium / high: how much code changes, and how far the code
  drifts from the TypeScript structure (see the divergence discussion: decoder internals are
  cheap to diverge, finalize and the fold are not).

## Starting point (2026-11-10, idle machine)

- 5% sample: 57.2 s elapsed (60.1 s before the purge delay), peak memory 6.1 GB.
- Full archive: 18 min 43 s, peak 13.4 GB (before the purge delay; not measured since).
- Profile of the 5% run (self time): `Object::get` 17.6%, decoded-tree allocation and freeing
  ~18% (`drop_in_place` 6.9%, mimalloc pages ~8%, `TableBuilder::into_entries` 2.9%),
  `Cursor::item` 8.6%, `Decoder::map` 6.8%, memcpy/memset ~10% (a third of it the inflate copy),
  SipHash ~2%.

## Experiments, in the planned order

CPU by profile weight first, then memory where it is cheap, then the structural ones. Full-archive
Rust runs after the CPU batch, after the memory batch, and at the end. The order changed as the
profiles came in; see the results for what actually ran.

| # | Experiment | Why | Expected | Complexity |
|---|---|---|---|---|
| M0 | Report observations and approximate bytes per field after a run (`--stats`) | Know what the fold holds before shrinking it | — | low |
| P1 | `Object::get` fast path for name keys (compare `&str` directly, no `JsKey` built) | 17.6% self time | −10% | low |
| P2 | Stream entry access (`entry_t`/`entry_tp`/`entry_v`): cheaper lookups of `t`, `tp`, `v` | Most `Object::get` calls come from entries | −5% | low |
| P3 | Build table entries in place (no `into_entries` copy, fewer allocations) | Allocation ~18% | −5% | low |
| P4 | CBOR lengths of 1/2/4 bytes read as integers (doubles only for 8 bytes) | `Cursor::item` 8.6% | −2% | low |
| P5 | FxHash instead of SipHash in observer maps and sets (unit state pairing, etc.) | SipHash ~2% | −2% | low |
| P6 | Borrow `export_string` when it has no escapes; avoid copying the inflate buffer | memcpy ~10% | −2% | low |
| R1 | Re-profile; re-rank what is left | | | |
| F0 | Full archive, Rust, after the CPU batch | Milestone | | |
| M1 | Threads: 16 (physical cores) vs 32 (logical) | ~110 MB of decoded data per worker | −1.5 GB, ±few % time | low |
| M2 | Shrink quest objectives (half the fold: two strings per observed match) | ~5.6 GB of ~11 GB at full scale | −GBs | medium |
| M3 | Summarize custom-merge fields whose merge can be folded exactly | Keep per-entity state instead of every observation | −GB | medium |
| P8 | Parallel finalize, mild version: the big independent merges side by side, TS order kept otherwise | 76 s serial at full scale | −1 min full | medium |
| P7 | Smaller `Value` (box rare variants, `&str` instead of `Cow` where possible) | Less memory traffic per decoded value | −?% | medium |
| P9 | Tune: `AHEAD_PER_THREAD`, purge delay | Re-balance after the above | ±% | low |
| P10 | Lazy events (only if the profile still shows eager event decoding high) | Expected 1–2% only | −1–2% | medium |
| — | `-C target-cpu=native` | Already measured: −2%, within noise; makes the binary unportable | rejected | — |

M0 result (5% sample): the fold holds ~616 MB, growing linearly with sessions (~11 GB at full
scale). Objectives are 51% of it (314 MB), then npc spawns 10%, npc reactions 8%, finishers 5%,
quest factions 4%, reputation 4%. Peak memory beyond the fold is decoded submissions in flight.

## Results

| # | Result | Time (5%) | Peak memory (5%) | Kept? | Commit |
|---|---|---|---|---|---|
| base | starting point | 57.2 s | 6.1 GB | — | ed77d0c |
| M0 | `--stats` per-field memory report | no change (off by default) | — | yes | 8780de9 |
| — | **re-baselined on the now idle machine** (M0 binary) | 43.9 / 44.8 s | 5.9–6.2 GB | — | — |
| P1 | `Object::get` fast path for name and index keys | 42.8 / 42.3 s (−3%) | 5.9–6.0 GB | yes | 2a0dc0b |
| P1b | Hash index of names for objects with >16 names (the ~100 function streams) | 45.2 / 45.6 s vs 42.9 / 45.3 s: no gain | +0.3 GB | **no** | — |
| — | Finding: 78% of `Object::get` time is one instruction, the first load of an object's keys: cache misses, not key handling. Observers walk every event ~8 times, chasing a pointer per event per pass | | | | |
| P2 | Read every event's name, `t`, `tp` once into a contiguous array when a session is created | 39.2 / 39.3 s vs 42.4 / 42.3 s (−7%) | 5.8–6.1 GB | yes | f4bb554 |
| P4 | CBOR header arguments read as integers (doubles only for 8-byte values above 2^53) | 38.7 / 38.6 s vs 39.5 / 39.7 s (−2%) | 6.0 GB | yes | acda299 |
| P3 | Finish decoded tables in place (`filter_map` collects into the slots' own allocation) | 37.9 / 37.9 s vs 39.4 / 39.1 s (−3%) | 6.0 GB | yes | 206e1b3 |
| P5 | FxHash instead of SipHash in observer maps/sets and the decoder's table index | 37.6 / 37.4 s vs 38.0 / 37.8 s (−1%) | 5.6–5.8 GB | yes | 525fc94 |
| P6 | Borrow `export_string` when it has no escapes; inflate into recycled buffers (no copy, no zero-fill) | 33.5 / 33.3 s vs 37.4 / 38.0 s (−11%) | 5.8 GB | yes | 590fa89 |
| R1 | Re-profile: decoding is ~55% (value building 16.5%, CBOR headers 13.5%, validation ~9%, alloc/free ~14%); eager event decoding is ~40% of value building — events are built but, after P2, barely read | | | | |
| F0 | **Full archive, CPU batch (P1–P6):** 11 min 49 s (was 18 min 43 s); fold thread busy 121 s, finalize 48 s; outputs identical to the first full run | — | **18.5 GB** (was 13.4 GB): recycled inflate buffers stay per thread, plus the purge delay | — | 590fa89 |
| P10 | Lazy events: an event stays undecoded; `Session::new` reads `e`/`t`/`tp` straight from its bytes (falls back to decoding for anything odd) | 28.7 / 28.5 s vs 33.1 / 33.8 s (−14%) | 5.4 GB | yes | 1c36e8d |
| M-a | Keep one spare inflate buffer per thread (the largest), not one per export of the largest submission seen | 28.8 / 29.0 s vs 28.6 / 28.8 s (+0.7%, noise) | 4.8–4.9 GB vs 5.2 GB (−8%) | yes | bfbb5d2 |
| M1 | Threads 24 / 16 instead of 32 (same binary, `--threads`) | 24: 30.5 / 31.1 s (+4.5%); 16: 34.4 s (+16%) vs 29.1 / 30.4 s | 24: 3.9 GB (−21%); 16: 2.7 GB (−45%) | **no**: ~2% and ~3.6% speed per 10% memory, over the 1% rule; `--threads` stays available | — |
| M2 | Objectives folded as per-session slots (kind, position, id): the merge never reads the matched name and text, and counts sessions per slot | 28.9 / 28.7 s vs 28.9 / 28.5 s (noise) | fold 616 → 319 MB (−48%); at full scale ~−5 GB | yes | 7dd06f3 |
| R2 | Re-profile: CBOR walking dominates (`Cursor::item` 18.4%: half the validation walk, the rest resolving streams and event headers); alloc/free ~12%; finalize is 77% npc spawn merges | | | | |
| P8 | Parallel finalize, mild: the per-entity spawn merges (npc and object) run on all cores, entities handed out one at a time, entity order kept; nothing else in finalize changes | finalize 0.9 → 0.3 s on 5% (full scale: see F1) | — | yes | 64346ce |
| P11 | `npc_drops`: look loot sources up by time in a map instead of scanning the source stream per link | 27.8 / 27.5 s vs 27.2 / 27.5 s: no gain | — | **no** | — |
| — | Rejected on inspection: parse JSON before the lossy UTF-8 pass (that pass is already the only validation; nothing to save) | | | — | — |
| P12a | Trusted walking of already validated bytes (event headers; shapes of lazy entries while a lazy value decodes): an item counter, no error checks, no recursion | 26.5 / 26.8 s vs 28.5 / 27.8 s (−5%) | 4.5–4.6 GB | yes | 7be09d1 |
| — | Noise check: the same binary on both sides gave 37.0–37.8 s, so differences under ~2% are within noise | | | | |
| F1 | **Full archive, everything through P12a:** 9 min 6 s (F0 11 min 49 s, −23%); fold thread busy 104 s (was 121 s), finalize **6.8 s** (was 48 s); outputs identical to F0 | — | **11.0 GB** (F0 18.5 GB, −41%) | — | 0df3ac2 |
| R3 | Re-profile (F1 binary): freeing the decoded tree is 7.7% (`drop_in_place`, 99% from the worker dropping a submission), mimalloc alloc paths ~8% more | | | | |
| A | **Arena:** decoded values in a per-worker bumpalo arena, reset after each submission; strings borrowed, arrays/objects arena slices, lazy results arena references; `Value` owns nothing (compile-time `!needs_drop` check). Deleted map keys are marked in a side vector allocated on first delete (stays linear) | **22.6 / 22.8 s vs 26.9 / 27.2 s (−16%)**, user −21% | uncapped: 6.8–6.9 GB (+45%): a reset arena keeps its largest block, so every worker pinned the largest submission it ever saw | yes, with the cap below | e9304f1 (branch `exp/arena`) |
| A-cap | Cap what a worker keeps between submissions (replace the arena when its kept block is larger). Per-submission arena: median 10 MB, p90 37 MB, p99 88 MB, max 165 MB. Caps 64 / 32 / 16 / 0 MB all ran 22.1–23.0 s | 16 MB: 22.1 / 22.4 s | 64: 6.2–6.6 GB; 32: 5.5–5.7 GB; **16: 4.9 GB**; 0: 5.0–5.3 GB (mimalloc's purge delay holds freed blocks anyway) | yes: 16 MB | e9304f1 |
| A-review | Fresh reviewer on the arena commit: no bugs; a 200k-case old-vs-new differential decode was identical. Two follow-ups fixed: an array with a hole reserved its declared length twice (hostile lengths), and `Value`/`Lazy` were `Clone` (a cloned lazy value would decode into the arena again) | no change | — | fix | c7537f9 |
| R4 | Re-profile (arena binary): 194k samples (was 248k); CBOR walking 19% (`Cursor::item`, half of it validation), UTF-8 checks of decoded strings ~5.7% (`Decoder::text` not inlined, plus `from_utf8`) | | | | |
| T1 | Inline `Decoder::text`, the lossy repair in a cold function | 21.8 / 22.6 s vs 22.8 / 22.1 s: noise | — | **no** | — |
| T2 | T1 plus an ASCII fast path (`from_utf8_unchecked` after `is_ascii`) | 22.4 / 21.8 s vs 21.4 / 22.1 s: no gain | — | **no** (and no `unsafe` needed) | — |
| P12b | Validation of nested items in one loop over a stack of open containers instead of recursion (same errors, same order); new seeded differential fuzz test (lazy vs full decode) and a lazy depth-limit test, both mutation-checked | 21.9 / 21.7 s vs 22.5 / 22.0 s (−1.5%), user −2% | 5.0 GB | yes | 85ec28b |
| P13 | Submission UTF-8: `from_utf8` (ASCII fast path) before falling back to `from_utf8_lossy` | 21.9 / 21.7 s vs 21.5 / 21.8 s: noise (user −1%) | — | **no** | — |
| P14 | Print decoding into a pre-sized (zero-filled) buffer instead of `extend_from_slice` per chunk | 22.1 / 22.0 s vs 21.7 / 21.7 s: slower | — | **no** | — |
| F4 | **Full archive, final binary (arena + P12b):** 7 min 22 s (F1 9 min 6 s, −19%; first run 18 min 43 s, −61%); user 10,698 s (F1 13,681 s, −22%); fold thread busy 101 s, finalize 5.9 s; outputs identical to F1 | — | **11.0 GB** (same as F1) | — | dcd87c5 |

## Final measurements

| # | What | Result |
|---|---|---|
| F1 | Full archive, Rust, best binary so far, under the memory watchdog | 9 min 6 s, 11.0 GB peak (546.6 s elapsed, 13,681 s user, 627 s sys) |
| F4 | Full archive, Rust, final binary (arena + iterative validation) | **7 min 22 s, 11.0 GB peak** (442.2 s elapsed, 10,698 s user, 638 s sys); outputs identical to F1 |
| F2 | Full archive, TypeScript tool from scratch (empty store, scratch directories; commit ab5f87f, the version the Rust parity is measured against) | **101 min 13 s, 18.8 GB peak** (6,054 s elapsed, 94,307 s user, 4,259 s sys; default 31 worker threads); store afterwards 3.0 GB |
| F3 | Diff the two full-archive outputs (parity on the whole archive, not only samples) | **identical**: all 42 files (41 modules + meta.json), timestamps aside |

## Outcome

Full archive (133,017 submissions, 655,876 sessions), same machine, idle:

| | Elapsed | CPU (user) | Peak memory |
|---|---|---|---|
| TypeScript, from an empty store (F2) | 101 min 13 s | 94,307 s | 18.8 GB |
| Rust, first full run | 18 min 43 s | — | 13.4 GB |
| Rust, final (F4) | **7 min 22 s** (13.7x faster than TS) | 10,698 s (8.8x less) | **11.0 GB** (−41% vs TS) |

Outputs are byte-identical to the TypeScript tool on the whole archive (F3), and on the 1% and 5%
samples after every kept change.

Kept, by gain on the 5% sample (complexity in brackets):

1. Arena per worker for decoded values, with a 16 MB retention cap: −17% (medium; decoder
   internals only)
2. Lazy events, headers read straight from the bytes (P10): −14% (medium)
3. Recycled inflate buffers and a borrowed export string (P6): −11% (low)
4. Event names and times read once per session into a contiguous array (P2): −7% (low)
5. Trusted walking of already validated bytes (P12a): −5% (low)
6. `Object::get` fast paths (P1), tables finished in place (P3), integer CBOR arguments (P4),
   FxHash (P5), iterative validation (P12b): −1% to −3% each (low)
7. Memory: objectives folded as per-session slots (M2, fold −48%), one spare inflate buffer per
   thread (M-a), parallel spawn merges in finalize (P8, finalize 48 s → 6 s at full scale)

Rejected (no gain or not worth it): name hash index (P1b), loot source map (P11), fewer threads
(M1, over the 1%-per-10% rule), inlined/ASCII-fast-path string checks (T1/T2), UTF-8 fast path for
submission files (P13), pre-sized print decoding (P14), `target-cpu=native`.

Not tried: M3 (summarize custom-merge fields in the fold). The fold is ~319 MB on the 5% sample
and grows with sessions, so it is most of the 11 GB at full scale; shrinking it means rewriting
each field's merge as an exact running summary — medium complexity, real parity risk, and memory
was the secondary goal.
