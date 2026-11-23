# Pack format, version 1

A pack holds many trace-data submissions in one object: one compressed frame per submission, then an index of the frames. This is the specification the writer and reader in this crate implement. Why packs exist, and how they are made and stored, is in the `railway-forms` repository's `OVH_MIGRATION_PLAN.md`.

What a reader gets from a pack is, for every submission, what it would get from the submission's file, with one permitted difference: in a session whose `tp` values are rounded, each float `tp` comes back as the nearest whole microsecond, written in the shortest float width that holds it. Everything else comes back byte for byte.

## Layout

```text
header     16 bytes   zstd skippable frame: "QTPK", format version
frame 0               one zstd frame: the record of one submission
frame 1
...
index                 one zstd frame: where each frame is, and what it holds
trailer    24 bytes   zstd skippable frame: where the index is
```

All integers in the header and trailer are little-endian.

| Part | Bytes | Contents |
| --- | --- | --- |
| Header | 0..4 | `50 2A 4D 18`: skippable frame magic `0x184D2A50` |
| | 4..8 | `8`: the length of what follows |
| | 8..12 | `QTPK` |
| | 12..16 | Format version, `1` |
| Trailer | 0..4 | `51 2A 4D 18`: skippable frame magic `0x184D2A51` |
| | 4..8 | `16`: the length of what follows |
| | 8..16 | Offset of the index frame from the start of the pack |
| | 16..24 | Length of the index frame |

- **Frames lie back to back.** The first starts at byte 16, each next one where the one before ends, and the index where the last one ends. The index ends where the trailer starts.
- **Every frame and the index carry zstd's content checksum** and state their decompressed size. A frame that was damaged fails to decompress.
- Because the header and trailer are skippable frames, `zstd -d` decompresses a whole pack: it yields every record, then the index, one after the other.
- A reader that has only part of a pack fetches the last 24 bytes, then the index, then the frames it wants.

A reader refuses a pack when any of this does not hold: the magic or length fields, the version, an index that does not end at the trailer, frames that are not back to back in index order, or bytes between header and index that the index does not list. It also refuses a frame that states a decompressed size above 1 GiB, or an index above 256 MiB, before decompressing it.

## What the frames hold

A record and the index are CBOR, written with definite lengths and each integer in its shortest form. `[...]` below is a CBOR array, `text` a text string, `bytes` a byte string.

### Index

```text
index = [ entry, ... ]                      one per frame, in frame order
entry = [ name:          text,
          offset:        uint,              of the frame, from the start of the pack
          length:        uint,              of the frame
          submission_id: text,
          contributor_id: text,
          received_at:   text,
          exports:       [ [ encoding: uint, sha256: bytes(32) ], ... ] ]
```

- `name` is the key the submission had in its source, such as `2026-11/2026-11-08_180449Z_<id>.json`.
- The three ids are read from the submission the way the extractors read them, so a reader can choose submissions without opening their frames. Free text in the file, such as the nickname, is not in the index.
- `exports` has one line per export, in order. `sha256` is of the export's CBOR as a reader gets it, after any rounding. A file kept whole has a single line with encoding 0 and the SHA-256 of the file.

### Record

```text
record  = [ 0, name: text, file: bytes ]                          a file kept whole
        | [ 1, name: text, head: bytes, tail: bytes, [ export, ... ] ]

export  = [ 1, cbor: bytes ]                                       exact
        | [ 2, skeleton: bytes, sessions, tp, t, x, y ]            value encoding
sessions = [ [ started_at: uint, t_count: uint ], ... ]
tp, t, x, y = [ int, ... ]                                         differences
```

**A file is split** when it is valid UTF-8, parses as a JSON object whose `export_string` is a string without escapes, that string holds at least one export marker, and every export in it inflates. Then:

- `head` is the file up to and including the quote that opens the export string.
- `tail` is the file from the quote that closes it.
- `head` followed by `tail` is the file with an empty export string. A reader reads the ids from that text exactly as it reads them from a file.
- Each export is the CBOR it inflated to. Text in the export string outside the export markers is not kept.

**Any other file is kept whole**, and a reader reads it through the same code as a file on disk. It therefore fails, or partly succeeds, in the same way and with the same messages.

An export is stored exact when the extractors' decoder rejects it, when the value encoding finds nothing to convert, or when its value encoding does not pass the check below.

## Value encoding

In an export's CBOR, some floats under the keys `tp`, `t`, `x` and `y` are replaced by a one-byte placeholder. Their integer forms are kept in four streams, one per key, in the order the placeholders appear.

| Key | Placeholder | Integer | Reader computes |
| --- | --- | --- | --- |
| `tp` | `E0` | `round(tp × 1000000)` | `integer / 1000000` |
| `t` | `E1` | `round((t + startedAt) × 1000)` | `integer × 0.001 − startedAt` |
| `x` | `E2` | `round(x × 10000)` | `integer / 10000` |
| `y` | `E3` | `round(y × 10000)` | `integer / 10000` |

- The placeholders are the CBOR simple values 0 to 3. The addon's serializer does not write them and the extractors' decoder rejects them, so an export that decodes cannot contain one.
- A key is one of the four when it is a text or byte string with those contents, however its length is written. The extractors' decoder reads both kinds of string as the same key.
- Arithmetic is on 64-bit floats. For `t`, the multiplication and the subtraction are two separate operations, never fused: the formula repeats what the addon did, `GetTime() − startedAt`, where `GetTime()` is a count of milliseconds times 0.001.
- `round` rounds half away from zero. Only the writer rounds, so the choice affects which values convert, not what a reader returns.
- The reader writes every float it restores in the shortest of CBOR's three float widths that holds the value exactly, as the addon's serializer does.

### Which values are converted

Only inside a session, which is a map in the array under the root's `sessions` key. Only floats: a value written as an integer is never touched.

**`t`, `x`, `y`**, each value on its own. A value is converted when all of these hold:

1. It is finite, and its integer is at most 2^53 in size.
2. The reader's formula gives back the same 64 bits. Negative zero therefore never converts.
3. Written in its shortest width, it has the width it had in the export.

For `t`, the session must also have a finite number as its `startedAt`. An integer serves.

**`tp`**, all of a session's float `tp` values or none. They are converted when all of these hold:

1. The session's `schemaVersion` is one the writer was told to round. This is how rounding stays limited to trace versions an extractor parity run has covered.
2. Every map in the session with a numeric `t` also has a numeric `tp`, and every float `tp` is in such a map.
3. Every `tp` is finite, its integer is at most 2^53 in size, and none is negative with an integer of zero: that one would come back as positive zero.
4. Take every numeric `tp` in the session, integers included, and the session's `durationPrecise` when it is a number, which must then be finite. After rounding the float `tp` values, every pair compares as it did before: less stays less, equal stays equal.

The whole export is unsuitable, and stored exact, when a map holds one of the four keys twice, when a session holds `startedAt`, `durationPrecise` or `schemaVersion` twice, when the root holds `sessions` twice, or when `sessions` is not an array.

### Streams

- A stream holds its values as differences: the first value, then each value minus the one before it.
- `sessions` has one row for each session with converted `t` values, in order: the 64 bits of its `startedAt` as an unsigned integer, and how many `t` values it has, which is at least one. The reader takes the start time for each `t` placeholder from the row it falls in.
- Every stream value must be used by exactly one placeholder, and every row by its count of `t` values. A reader fails otherwise.

### Reading

One pass over the skeleton's items. A string is stepped over by its length; every other item is a header, and what it contains follows as further items, so no nesting has to be tracked. At a placeholder the reader takes the next value of that stream and writes its float.

## What the writer checks before a frame is used

A frame is decompressed and decoded with the reader, and compared with the file it was made from:

- A file kept whole must be the file.
- `head`, `tail` and the number of exports must be as written.
- An exact export must be the inflated CBOR, byte for byte.
- A value-encoded export is compared with the inflated CBOR item by item. Every item must be byte-identical, except a float directly under a `tp` key inside a session. That one may instead be `round(tp × 1000000) / 1000000` in its shortest width.
- A session in which any `tp` differs must satisfy all four `tp` rules above, with every one of its float `tp` values rounded.

A value-encoded export that fails is stored exact and the frame is built again. Any other failure is an error, and the pack is not written.

The comparison is implemented apart from the encoder. It applies the rules to the original export itself and recomputes each rounding from the original value; it does not use the encoder's record of what it converted. The two share only the list of allowed schema versions and the routine that writes a float in its shortest width, which is tested against the extractors' decoder for every half float and a large sample of the others.

After the last frame, the writer opens the finished pack with the reader: the index must be the one it wrote, and every frame must be the bytes it was given.

## What is not kept

- The export string itself: its DEFLATE stream, its print encoding, its markers and any text between exports. The CBOR it carried is kept.
- The digits of `tp` below one microsecond, in sessions whose `tp` is rounded.
- The float width of a `tp` in such a session, when its rounded value fits a shorter one.

## Numbers

One hour of real submissions, received 2026-11-08 18:00 to 19:00 UTC: 1,490 submissions, 3,129.7 MiB of JSON, 10,678.4 MiB of CBOR, zstd level 9.

| Encoding | Pack | Of JSON |
| --- | --- | --- |
| Exact | 1,946.8 MiB | 62.2% |
| Value, `tp` left as floats | 1,262.3 MiB | 40.3% |
| Value, `tp` rounded | 791.1 MiB | 25.3% |

- Every export passed the check; none fell back to exact.
- Converted: 99.81% of `tp` floats, 99.84% of `t`, 96.90% of `x` and of `y`.
- `tp` was rounded in 8,319 of 8,331 sessions. The other 12 would have had two values compare differently.
- Six submissions were kept whole because one of their exports could not be inflated.
- Building took about 45 seconds on 32 cores, including the check.
