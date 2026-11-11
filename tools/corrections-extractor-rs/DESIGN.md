# corrections-extractor-rs: design

A Rust port of `tools/corrections-extractor`. It reads every submission of a local trace-data
checkout and writes the same output files: `forever{Npc,Quest,Item,Object}Traces.lua`,
`xpDB-traces.lua`, `lookup{Npcs,Quests,Items,Objects}/<locale>.lua` and `meta.json`.

The TypeScript tool is the reference. Until it is retired, every change here is checked by
running both on the same sample and diffing the outputs (see `scripts/parity.sh`).

## Why a port, and what changes

Measured on the TypeScript tool (2026-11):

- Decoding (print alphabet, raw inflate, CBOR into JS objects) is ~88% of the time spent on a
  submission; all observers together are ~8%.
- A 5% sample (6,760 submissions) took 19 min: ~14.5 min observing, most of it outside the
  observers (worker messaging, DuckDB batch writes), and ~4 min folding and writing.

The port keeps the observable behaviour and changes how it is computed:

| TypeScript | Rust |
|---|---|
| Decodes every session into JS objects | Decodes lazily: a function stream when first read, and then only the stream entries read (a binary search over time probes a few); event arguments when first read |
| Worker threads, results posted to the main thread | Worker threads claim submissions one at a time; one thread folds them in order, the workers at most a few submissions per thread ahead |
| Incremental DuckDB/parquet store between runs | None: every run is a full scan of a local checkout |
| Observations as JSON values with string keys | Typed observations per field |
| Scalar observations summarized on the main thread | Pre-merged per session and value on the worker that observed them |
| Raw custom-merge observations on disk | Kept in memory, compact and typed |

Source: a local trace-data checkout only (`--trace-data` or `TRACE_DATA_DIR`). Runs against the
OVH bucket sync it first.

Each worker decodes a submission into its own arena (bumpalo) and resets it once the
submission is observed: decoded values own nothing, so a submission's tree is freed at once
instead of value by value. A worker keeps at most 16 MB of arena between submissions, so one
huge submission does not stay pinned for the rest of the run. Everything else (observations,
the fold) uses mimalloc as the global allocator: with the system allocator, threads contended
on it and used twice the memory.

## Layout

```
crates/
  trace-data/    Reading submissions: listing and sampling, the JSON envelope, export markers,
                 print decoding, inflate, CBOR, the Lua-like Value tree, lazy Session view,
                 stream lookups (value_at, nearest_by_time). Knows nothing about corrections.
  corrections/   The extraction: observers (one module per field), the fold (session order and
                 dedupe), scalar summaries, merges, finalize, Lua writers, lookups, CLI.
scripts/
  parity.sh      Runs both tools on a sample and diffs their outputs.
```

## Pipeline

1. **List** `submissions/<yyyy-mm>/*.json` as keys `<yyyy-mm>/<file>.json`, apply the sample
   (`--sample` / `EXTRACT_SAMPLE`: first 4 bytes of SHA-256(key) as a fraction below the
   percentage) and sort newest first. The position in that order is the submission's rank.
2. **Observe** on worker threads, one submission at a time. Each submission: read the
   envelope, decode every export, and per session compute its key and, for Forever sessions,
   the staged observations of every observer (scalar ones already merged per value). Sessions
   whose key a folded submission used are skipped without observing.
3. **Fold** in rank order on one thread, as results arrive. Of each session key only the first
   copy met is used (newest submission, then export order); sessions without a key are always
   used. Scalar fields go into per-value summaries; custom-merge fields keep every observation.
4. **Finalize** merges facts per field, builds records and writes the Lua modules, then
   `meta.json` last (it marks a complete run).

## Behaviour that must match the TypeScript tool exactly

These decide the output bytes; each one was a parity bug or would be.

- **Value shapes.** The CBOR decoder normalizes tables the way `core/trace-data/cbor.ts`
  does: a table whose keys are exactly `1..n` is an array unless its first value is itself a
  table with only positive integer keys; nil values delete keys; boolean/table keys are
  dropped; an empty table is an empty array. `Value::is_array()` reports that shape. Code that
  ported `Array.isArray(x)` must use it.
- **Key order.** JS objects iterate integer-like keys (canonical, `0 ..= 2^32 - 2`) ascending,
  then other keys in insertion order. `Table::entries()` yields that order. Observers that
  ported `Object.keys`/`Object.entries` must use it.
- **Keys are strings in JS.** `getStream(session, name, 100)` finds key `100` and key `"100"`.
  `Table::get` accepts both.
- **Numbers.** Every CBOR number is an `f64`. Output formats numbers like JS `String(n)`
  (`ryu-js`). Integers are compared as `f64`.
- **Map and Set order.** Where TS iterates a `Map`/`Set` and the order reaches the output
  (or a stable sort's tie-break), use `IndexMap`/`IndexSet` in the same insertion order.
- **Sorting.** JS `Array.prototype.sort` is stable: use `sort_by` (stable), never
  `sort_unstable`, when ties can occur. Default JS sort without a comparator compares as
  strings.
- **Scalar summaries** keep value groups in first-met order; ties in the default order resolve
  to the earlier group (stable sort).
- **Session identity.** Session keys are compared for equality only, so a session is
  identified by its two start clocks (`-0` treated as `0`). Sessions without both clocks get
  their own identity, numbered by fold order (`#<n>` in TS).

## Known differences

Reviewed and left as they are: real trace data cannot produce any of them.

- A submission whose `export_string` holds a lone UTF-16 surrogate escape (`\ud800`) is read as
  "no export_string"; JS strings allow lone surrogates, Rust strings do not.
- An entry without `t` reads as NaN here and as NULL in the TS tool's DuckDB summary, so
  `latestT` can differ for it. The addon always records `t`.
- An observer that panics stops the run even on a session copy that would be skipped as a
  duplicate; the TS tool only stops when the failing copy is used.

## Observers

One module per field under `corrections/src/observers/<entity>/`, holding the field's value
type, its observer and (for custom fields) its merge, next to their tests. An observer is a
plain function `fn(&Session) -> Vec<Observation<T>>` and must not hold state between
sessions. Shared helpers keep the names of their TS counterparts (`probes`, `correlate`,
`guid`, `item_link`, `_encounters`, ...).

Tests are ported from the `.test.ts` next to each observer, keeping their expectations. A test
that cannot be ported 1:1 says why.
