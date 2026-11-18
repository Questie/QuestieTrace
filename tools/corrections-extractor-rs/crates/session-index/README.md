# session-index

Lists every recorded session of a trace-data checkout once, and says which submission files hold
nothing that is not kept from another file. **Read-only**: it never changes the checkout, it writes
what it found for cleanup scripts to act on.

One session reaches trace-data many times. An export holds every saved session plus the one in
progress, and a session stays in the player's SavedVariables until they confirm the submission
(older addon versions never removed it). So the same session comes in again with every later
export: repeated as it was, or longer.

## Running it

```sh
cd tools/corrections-extractor-rs
cargo run --release -p session-index -- --trace-data ../../../trace-data --output session-index-output
```

| Option | Default | |
|---|---|---|
| `--trace-data DIR` | `$TRACE_DATA_DIR` | The trace-data checkout to read (`submissions/<yyyy-mm>/*.json`) |
| `--output DIR` | `session-index-output` | Where the index goes (gitignored) |

Decoding uses every core (`RAYON_NUM_THREADS` to use fewer). The full archive (199,967
submissions, 392 GB) takes about 8 minutes and 4 GB of memory.

## Output

`sessions.jsonl`: one line per session, its kept copy and every other (superseded) copy:

```json
{"startedAt":319817.935,"startedAtPrecise":319817.9361147001,
 "kept":{"submission":"2026-09/2026-09-21_235258Z_ec69….json","export":0,"session":0,"events":5,"functionBytes":2102,"stopped":true},
 "superseded":[{"submission":"2026-09/2026-09-21_232240Z_db3c….json","export":0,"session":0,"events":5,"functionBytes":2102,"stopped":true}]}
```

A copy is addressed by its submission file (under `submissions/`), the position of its export among
the export markers of `export_string`, and its position in that export's `sessions` array (both
0-based). `startedAt`/`startedAtPrecise` are null for a session without both clocks.

`submissions.jsonl`: one line per submission file, with its `status`:

| Status | Meaning |
|---|---|
| `redundant` | Every export decoded, every record in it is a valid session, and every one of them is superseded by a copy kept from another file |
| `needed` | Holds a kept copy, or something this index cannot read: an export that did not decode (`droppedExports`) or a record that is no valid session (`invalidSessions`) |
| `empty` | Every export decoded and none holds a session record |
| `unreadable` | Nothing in it could be decoded (`error`) |

`summary.json`: the totals, also printed. It is written last, so its presence marks a complete index.

## Which copies are one session

Copies of a session share its capture-start clocks (`startedAt`, `startedAtPrecise`; what the
extractors match copies by) and its first event (name, `t`, `tp` to the microsecond). The first
event keeps sessions apart whose clocks happen to agree (`sessionsSharingClocks` counts them). The
microsecond is needed because a saved copy went through SavedVariables, which keeps fewer digits:
`tp` 0.0057977999999820895 came back as 0.00579779999998209.

Of a session's copies, the one kept is, in order: the one with the most events; a stopped one (a
stopped session never records again); the one with the most function data (sampling goes on
between events); the one from the newest submission (exports were cumulative, so this leaves the
older files redundant); the first in that submission.

## What it does not check

Copies are matched, not compared: a superseded copy is not checked to be a prefix of the kept one.
Probes on real data found none that was not: 25,169 superseded copies over two days compared event
by event, and every submission of the five most active contributors compared stream by stream (one
pair differed, in float digits SavedVariables dropped). A script that deletes files should still
compare what it deletes with the kept copy first.

`redundant` judges sessions only. Deleting a file also loses its envelope (`contributor_id`,
`received_at`, `nickname`).

## The archive on 2026-11-13

199,967 submissions held 1,138,164 session copies of 1,030,087 sessions. Of the 108,077
superseded copies, every one has as many events as its kept copy; 6,223 hold less function data
(6,205 in-progress copies of a session whose stopped copy came in too, 18 stopped copies). 10,600
submissions are redundant, 37 unreadable, 4 records are no valid session, and no two sessions share
their clocks.
