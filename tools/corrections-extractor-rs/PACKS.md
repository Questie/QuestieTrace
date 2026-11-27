# Packs: every tool, and how to run it

The trace archive is moving from one JSON file per submission to **packs**: many submissions in one compressed, indexed file, at about a quarter of the size. This page lists every tool that makes, reads or checks packs, with the command to run it. Each tool's own page has the details.

| Tool | Crate | What it does | Its page |
| --- | --- | --- | --- |
| `trace-compactor backfill` | `trace-compactor` | Packs the old archive (the old bucket, a local checkout, or both) into a local directory | [README](crates/trace-compactor/README.md#backfill) |
| `scripts/backfill-days.sh` | | Runs the backfill one day at a time, for a run left alone for hours | its header |
| `trace-compactor run` | `trace-compactor` | The hourly compactor on the server: packs the archive bucket's `loose/` and answers removal requests | [README](crates/trace-compactor/README.md) |
| `corrections-extractor-rs` | `corrections` | The corrections extractor; reads a checkout or packs | [README](README.md) |
| `relations` and `scripts/relations.sh` | `relations` | The quest relations pipeline; reads a checkout or packs | [README](crates/relations/README.md) |
| `trace-pack build`, `verify`, `list` | `trace-pack` | Builds packs from a checkout, checks packs against it, prints a pack's index | [FORMAT.md](crates/trace-pack/FORMAT.md) |

Build them all with `cargo build --release` in this directory. The binaries are in `target/release/`.

## Where traces are read from

Every tool that reads traces takes one directory, as `--trace-data` or `TRACE_DATA_DIR`:

- **A checkout:** it holds `submissions/<yyyy-mm>/*.json`.
- **Packs:** it holds `packs/`, with `.pack` files at any depth.

A directory with both is refused. A tool reads either kind the same way: the same submissions in the same order, and the same output. Both extractors were compared over 199,967 submissions on 2026-11-25; see [REQUIRED_EXPORTER_CHANGES.md](../../REQUIRED_EXPORTER_CHANGES.md#parity-run).

**Packs on several disks** are read as one directory of links:

```sh
mkdir -p all/packs
ln -s /disk-a/trace-packs/packs all/packs/a
ln -s /disk-b/trace-packs/packs all/packs/b
TRACE_DATA_DIR=all ...
```

A link that leads nowhere, such as to a disk that is not mounted, is an error, never a smaller archive. A submission that is in two packs is read once.

## Pack the old archive

From a local checkout only:

```sh
target/release/trace-compactor backfill --out PACKS --trace-data CHECKOUT
```

From the old bucket, with a local checkout used wherever it holds the same object. The bucket is only listed and read:

```sh
export SOURCE_S3_ENDPOINT=s3.de.io.cloud.ovh.net SOURCE_S3_REGION=de SOURCE_S3_BUCKET=trace-intake-ovh
export SOURCE_S3_ACCESS_KEY_ID=... SOURCE_S3_SECRET_ACCESS_KEY=...   # a key that may only read
target/release/trace-compactor backfill --out PACKS --trace-data CHECKOUT \
  --only 2026-11/2026-11-24 --download-mb-per-second 40 --threads 24
```

For many days, unattended, with retries, a free-space limit and a log in `PACKS/backfill-days.log`:

```sh
BIN=target/release/trace-compactor OUT=PACKS ALSO_PACKED=OTHER_PACKS RCLONE_CONF=rclone.conf \
  MB_PER_SECOND=40 THREADS=24 BUILD_INPUT_MIB=128 \
  scripts/backfill-days.sh 2026-11-24 2026-11-30
```

- `ALSO_PACKED` names a directory whose packs count as packed, for an archive on two disks. Leave it out for one.
- `RCLONE_CONF` is an rclone config with one S3 remote; its key is read from it and never put on a command line.
- One thread downloads about 1.5 MB/s of small objects, so reaching 40 MB/s takes about 24 threads. `CPUS` sets the processors they may use, and that is what bounds the load.
- A run packs only hours that ended 48 hours ago, skips what is already packed, and can be stopped and started again at any moment.

## Read packs

```sh
target/release/corrections-extractor-rs --trace-data PACKS --output OUT
QUESTIEDB_DIR=QUESTIEDB TRACE_DATA_DIR=PACKS scripts/relations.sh
```

## Check packs

| Check | Command |
| --- | --- |
| Packs against the checkout they were made from: every record reads back as its file, and every submission is in exactly one pack | `target/release/trace-pack verify --trace-data CHECKOUT --packs PACKS` |
| One pack's index | `target/release/trace-pack list PACK` |
| Each pack is the file it was written as: its MD5 is the name after the hour | `md5sum PACK`, against `HH-<md5>.pack` |
| Nothing of the old bucket is missing | a run of `trace-compactor backfill` without `--only` packs nothing and reports nothing gone |

`trace-pack list` reads the whole pack. Anything that only needs the names reads the index alone: the pack's last 24 bytes say where it is.

## Upload packs

A sync tool uploads `PACKS/packs`, and **must upload each pack in one part**: a removal can only rewrite a pack whose ETag is its MD5. With rclone, set `use_multipart_uploads = false` on the remote. The details are in the [compactor's README](crates/trace-compactor/README.md#backfill).

## `tp`

Packs store `tp` rounded to whole microseconds in every session where that changes no comparison between its `tp` values, whatever its `schemaVersion`; this has been the default since 2026-11-27. `--exact-tp` keeps every `tp` as recorded, and `--tp-schema N,N,...` limits rounding to those versions. Why, and what was compared, is in [REQUIRED_EXPORTER_CHANGES.md](../../REQUIRED_EXPORTER_CHANGES.md#parity-run).
