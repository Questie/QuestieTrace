# corrections-extractor-rs: optimization plan

Overnight work on the Rust prototype in `tools/corrections-extractor-rs` (local only, never pushed).
Goal: find out which optimizations pay off, by how much, and at what complexity; then measure the
full archive with both tools.

## Decisions (from David, 2026-11-10)

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

## Experiments, in order

CPU by profile weight first, then memory where it is cheap, then the structural ones. Full-archive
Rust runs after the CPU batch, after the memory batch, and at the end.

| # | Experiment | Why | Expected | Complexity |
|---|---|---|---|---|
| M0 | Report observations and approximate bytes per field after a run (`--stats`) | Know what the fold holds before shrinking it | — | low |
| P1 | `Object::get` fast path for name keys (compare `&str` directly, no `JsKey` built) | 17.6% self time | −10% | low |
| P2 | Stream entry access (`entry_t`/`entry_tp`/`entry_v`): cheaper lookups of `t`, `tp`, `v` | Most `Object::get` calls come from entries | −5% | low |
| P3 | Build table entries in place (no `into_entries` copy, fewer allocations) | Allocation ~18% | −5% | low |
| P4 | CBOR lengths of 1/2/4 bytes read as integers (doubles only for 8 bytes) | `Cursor::item` 8.6% | −2% | low |
| P5 | FxHash instead of SipHash in observer maps and sets (unit state pairing, etc.) | SipHash ~2% | −2% | low |
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

## Final measurements

| # | What |
|---|---|
| F1 | Full archive, Rust, final binary, under the memory watchdog: time and peak memory |
| F2 | Full archive, TypeScript tool from scratch (empty store, scratch directories): time and peak memory |
| F3 | Diff the two full-archive outputs (parity on the whole archive, not only samples) |

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
