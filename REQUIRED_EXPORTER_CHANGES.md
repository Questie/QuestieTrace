# Required exporter changes for packs

Status: **proposal**, written 2026-11-23. The pack reader exists and is tested (`tools/corrections-extractor-rs/crates/trace-pack`). Nothing below is done yet, and none of it starts before packs are being produced on the server: until then every tool keeps reading the JSON files.

The trace archive is moving from one JSON object per submission to **packs**: one object per hour, holding about a thousand submissions, at about a quarter of the size. The format is in [FORMAT.md](tools/corrections-extractor-rs/crates/trace-pack/FORMAT.md); the migration is planned in the `railway-forms` repository (`OVH_MIGRATION_PLAN.md`). This page lists what each tool that reads traces has to change.

## What changes for a reader

| | Today | With packs |
| --- | --- | --- |
| Where traces are | `submissions/<yyyy-mm>/<stamp>_<id>.json`, one file each | `packs/.../HH-n.pack`, about 1,000 submissions each |
| Listing | Every file | Each pack's index: name, ids and frame position of every submission |
| Reading one | Parse JSON, print-decode, inflate, decode CBOR | Decompress one frame, rebuild the CBOR, decode CBOR |
| Freshness | As soon as a trace is accepted | About 48 hours after it is accepted |

What a tool gets for a submission is the same as from its file, with these exceptions:

- **`tp` is rounded to a whole microsecond** in sessions where that changes no comparison between the session's `tp` values. `t`, positions and everything else are bit-exact.
- **The pasted export string is gone.** Its CBOR is kept; the DEFLATE stream, print encoding, markers and any text between exports are not.
- **A rounded `tp` can change a session's encoded size by a few bytes**, when the rounded value fits a shorter float. Anything that compares encoded sizes sees that.

A file the extractors cannot split today (not JSON, an export that does not inflate) is kept whole in the pack and read through the same code, so it fails in the same way with the same message.

## The reader

```rust
let bytes = std::fs::read(pack_path)?;                 // or a memory map
let pack = trace_pack::pack::PackReader::open(&bytes)?;
for (index, entry) in pack.entries().iter().enumerate() {
    // entry.name is the key the submission had as a file: "2026-11/<stamp>_<id>.json".
    // entry.submission_id, entry.contributor_id, entry.received_at need no frame read.
    let submission = pack.record(index)?.submission(&mut inflater)?;
    let decoded = submission.decode(&arena);            // exactly as after Submission::read
}
```

`submission` is the same `trace_data::submission::Submission` the extractors already use.

## Rust tools

### Corrections extractor (`crates/corrections`)

Reads through `list_submissions` and `Submission::read` in `src/run.rs` (lines 84 to 85 and 168 to 170).

1. **A source that lists packs.** Replace `list_submissions(&options.trace_data)` with a listing that accepts either a trace-data checkout or a directory of packs, and yields the same `key` for each submission. It has to live in `trace-pack` or above it, because `trace-pack` depends on `trace-data` and not the other way round.
2. **Keep the order.** The extractor walks submissions newest first, by key, and uses the first copy of each session it meets (`used` in `run.rs`). Packs are grouped by the hour a submission entered the archive, which is not the hour it was received, so pack order is not key order. The listing must read every pack's index, sort all entries by key, and only then read frames.
3. **Read by entry.** Replace `Submission::read(&file.path, inflater)` with the pack read above. With all packs memory-mapped, reading entries in key order across packs costs nothing extra.
4. **Sampling is unaffected.** `in_sample(&file.key, percent)` hashes the key, and the key is the entry's name.
5. **`meta.json` is unaffected**, as long as the key is passed through as it is today.

### Relations pipeline (`crates/relations`)

Reads through `list_submissions` in `src/ingest/mod.rs` (lines 73 to 76) and distils each `SubmissionFile`. The same three changes: list packs, keep key order, read by entry.

It is also the tool most sensitive to `tp`:

- It computes windows in seconds as `tp` differences. A threshold on such a difference can flip when the difference sits within a microsecond of the limit.
- `episodes.jsonl` prints `tp`-derived times at full precision, so that file will differ in its last digits. It can only be compared within one microsecond, not byte for byte.

### Parity run

Before any trace is stored with rounded `tp`, both tools are run twice over the local checkout: from the JSON files and from packs built from them.

- It passes when every final output is identical and intermediate `tp`-derived times differ by at most one microsecond.
- `scripts/parity.sh` and `scripts/relations-compare.sh` compare outputs today; the second needs a tolerance for `episodes.jsonl`.
- The run tells the pack writer which `schemaVersion`s may have `tp` rounded (`trace-pack build --tp-schema ...`). A version it did not cover keeps exact `tp`.

## TypeScript tools

| Tool | Reads through |
| --- | --- |
| `tools/corrections-extractor` | `src/core/trace-data/submissions.ts` (`readSubmission`) |
| The copy in the Exporters repository, with its relations pipeline | `tools/corrections-extractor/src/...`, `src/relations/ingest/submission.ts` |
| `tools/trace-analyzer` | Not checked yet |

They cannot read a pack without a zstd binding and a port of the value decoding. Two ways to go, and a decision to make:

1. **Do not port.** They keep reading the JSON checkout while the old bucket exists, and are retired with it. This fits if the Rust extractor becomes the primary one.
2. **A bridge: `trace-pack unpack`.** A command that writes a submission back out as a JSON file with a freshly deflated and print-encoded export string. Any existing tool reads that file; its contents equal the original up to the `tp` rounding. It does not exist yet. It is cheap for one trace or a sample, and costs the full uncompressed size for everything.

The trace analyzer opens one trace at a time, so option 2 for a single submission is probably all it needs.

## Syncing and docs

- **Local copy:** sync `packs/` from the new archive bucket, not `submissions/` from `trace-intake-ovh`. Packs do not change once written, except when a removed submission is cut out of one; a sync by checksum picks that up.
- **ADR 0002** ("raw data stays in the bucket") needs revising: the bucket will hold packs, and the old bucket is retired after the migration.
- **`docs/data-flow.md`** and the two extractor READMEs describe the checkout layout and need the pack layout beside it.

## Open questions

1. Which tools must read packs: only the Rust ones, or the TypeScript ones as well?
2. Is `trace-pack unpack` wanted, for the trace analyzer and for looking at a single trace by hand?
3. Does anything compare the encoded size of a session and need to know it can shift?
