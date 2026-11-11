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

## Final measurements

| # | What | Result |
|---|---|---|
| F1 | Full archive, Rust, best binary so far, under the memory watchdog | 9 min 6 s, 11.0 GB peak (546.6 s elapsed, 13,681 s user, 627 s sys) |
| F2 | Full archive, TypeScript tool from scratch (empty store, scratch directories): time and peak memory | pending |
| F3 | Diff the two full-archive outputs (parity on the whole archive, not only samples) | pending |
