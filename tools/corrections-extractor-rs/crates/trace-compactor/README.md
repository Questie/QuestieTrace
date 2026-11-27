# trace-compactor

Turns the accepted submissions in the archive bucket into packs, and removes the ones the intake asks it to. It is the only program that writes packs, and the only one that deletes anything in the archive bucket.

The pack format is [trace-pack](../trace-pack/FORMAT.md). The intake, which fills the bucket and writes the removal requests, is in the `railway-forms` repository: its `docs/storage.md` describes its side of the same bucket, and `OVH_MIGRATION_PLAN.md` why any of this exists.

This page is the description of record for what the compactor does and in which order.

## One run

The compactor is started once an hour and does one run:

1. **Finish** the pack an earlier run recorded and did not complete.
2. **Remove**: work on every removal request that has no answer yet.
3. **Pack** every loose submission the bucket stored longer ago than the settle window.

It keeps no state outside the bucket. Whatever it is about to destroy it first records there, so a run that stops anywhere, with a crash or a request whose answer never arrives, is finished by the next one.

**Only one run may work on a bucket at a time, whatever its writer name.** A removal rewrites packs of every writer, with what that run read of them. On the server, systemd does not start a run while one is going. A backfill under another writer name has to be finished, or stopped, while the server's compactor runs.

## What is where in the bucket

| Key | Written by | Holds |
| --- | --- | --- |
| `loose/<name>` | Intake | One accepted submission as a JSON object. `<name>` is `<month>/<time received>_<submission id>.json`. |
| `packs/<writer>/YYYY/MM/DD/HH-<md5>.pack` | Compactor | A pack: the hour of the run that wrote it, in UTC, and the checksum it had then. |
| `intents/<writer>/pack.json` | Compactor | The pack that is being stored, while it is. At most one per writer. |
| `removals/requested/<id>` | Intake | A request to remove a submission. Never changed or deleted by the compactor. |
| `removals/started/<id>` | Compactor | The record that the removal began, and where the submission was found. Kept for good. |
| `removals/done/<id>` | Compactor | The answer: the submission is gone. The intake then forgets it, which cannot be undone. Kept for good. |

`<writer>` is `live` for the compactor on the server. Another writer, such as a backfill, has its own packs and its own intent, so the two cannot write the same key.

A submission is identified by the id in its name, in `loose/` and in a pack's index alike, and never by its content: the same export sent by another contributor is another submission.

## Packing

For each pack, in this order:

1. **Build** the pack in memory from the loose objects. Each downloaded object must have the checksum the listing gave. Each frame is read back and compared with its object by trace-pack before it is used.
2. **Record the pack intent**: the pack's key, its checksum, and each loose object it holds with that object's ETag.
3. **Upload** the pack in one request that carries its MD5, which the provider enforces. Then ask the bucket for the stored object's ETag: it must be that MD5.
4. **Delete** each recorded loose object, only while its ETag is still the recorded one. Then delete the intent.

What this gives:

- **A submission is in `loose/` or in a confirmed pack at every moment.** Nothing is deleted before step 3 has confirmed the pack that holds it.
- **Two different packs never share a key**, because the key ends in the pack's checksum. A listing that lags or an upload that arrives late cannot put one pack over another, and an upload that is repeated stores the same bytes.
- **An intent whose pack is absent is discarded.** Nothing was deleted yet, so every loose object is selected again. Should that upload still arrive, its submissions are in two packs, which readers accept.
- **An intent whose pack is another object stops the run**, and every run after it, until someone has looked. So does a recorded loose object whose ETag changed. Neither can happen unless something else writes to the bucket.
- **One loose object that cannot be packed does not hold up the others.** It is left in `loose/`, reported, and the run ends as failed. That is an object that changed or vanished after it was listed, one that was not stored in one part, so that its ETag is not its MD5, or one whose frame does not read back.
- **A submission with a removal request is not packed.** Removing it from `loose/` is one delete; from a pack it is a rewrite.
- **A pack holds its submissions in the order they were received**, which is the order of their names. Which submissions share a pack follows the order the bucket stored them.

## Removing

A removal takes two runs. For each request without an answer, a run does one of two things:

**The submission is found**, in the listing of `loose/` or in the index of any pack of any writer:

1. **Record** that the removal starts, unless an earlier run already did.
2. **Delete** the loose object, only while its ETag is still the listed one.
3. **Rewrite** each pack that holds the submission without it, under the same key.

It writes no answer.

**The submission is found nowhere, and its removal was recorded as started:** write the answer, `removals/done/<id>`.

What this gives:

- **The answer always comes from a run that looked everywhere afresh and found nothing**, never from the run that deleted. An upload that was given up on and arrives late, or anything else that brought the submission back, is seen by that second look.
- **Finding the submission nowhere is not an answer by itself.** Without a started removal the request stays open and is reported, because that is also what a mistake in the search would look like.
- **Each pack's index is read with three small ranged requests.** An index that cannot be read stops the removal step: without it nothing can be said about where a submission is not. The packing still runs.
- **A rewrite replaces the only copy of every other submission in the pack**, so it is held to more:
  - The pack that is downloaded must have the checksum of the pack that was listed.
  - Frames are copied, never built again. Before anything is uploaded, every frame of the new pack is compared, byte for byte, with the frame the old pack has for that submission, and every index entry apart from the frame's position.
  - After the upload the bucket must hold the new pack's checksum, or the removal step stops.
- **A pack that loses its last submission stays, empty.** No pack is ever deleted.
- **A request that this version cannot read, or that names another id than its key, stays open** and is reported.
- **An interrupted pack is finished first.** Its intent names the pack by the checksum it had, which a rewrite would change.

## What a run reports

A run prints one line for each submission it removed, each request it answered, and each pack it wrote. It ends as failed, with a line for each, when:

- the removal step stopped (`REMOVALS STOPPED`);
- a removal request could not be worked on (`REMOVAL NOT ANSWERED`);
- a loose object could not be packed (`NOT PACKED`).

Each of those needs someone to look. Everything else the run could do, it has done.

An export that failed its check under the value encoding and was stored exact is printed as a warning under its pack. There should be none.

## Settings

```text
trace-compactor run [--writer NAME] [--settle-hours N] [--pack-input-mib N]
                    [--build-input-mib N] [--threads N] [--level N] [--exact]
                    [--tp-schema N,N,...] [--lock FILE]
```

The bucket comes from the environment, under the names the intake uses: `ARCHIVE_S3_ENDPOINT` (host only), `ARCHIVE_S3_REGION`, `ARCHIVE_S3_BUCKET`, `ARCHIVE_S3_ACCESS_KEY_ID`, `ARCHIVE_S3_SECRET_ACCESS_KEY`. The key must be allowed to read, write and delete in that bucket. It needs nothing in the intake bucket.

`tp` is stored rounded only for the `schemaVersion` values given with `--tp-schema`, which is for versions an extractor parity run has covered. Without it every `tp` stays a float.

### Memory

A pack is built and uploaded from memory.

| What | Needs about |
| --- | --- |
| Building frames | 22 times `--build-input-mib`. Measured on real traces: 23 MiB of JSON took about 480 MiB. |
| The pack being held | Between a quarter and two thirds of `--pack-input-mib`. |
| Rewriting a pack for a removal | Twice the pack's size. |
| Reading the pack indexes for a removal | One index at a time on each thread. Of what they list, only the requested ids are kept. |
| One submission larger than `--build-input-mib` | 22 times its own size. It is built alone. |

The last row has no bound short of the largest export the intake accepts. A submission whose build needs more than the machine has ends the run each time, before anything of that pack is written, and has to be packed on a larger machine.

Measured on 2026-11-25 on one real hour, 1,307 submissions and 3,075 MiB of JSON in two packs, with 8 threads, `--build-input-mib 64` and `--pack-input-mib 2048`: 2.8 GiB at its peak. Before frames gave up the room zstd reserves for them, the same run needed 6.4 GiB. The rows are rules of thumb and have not been measured one by one.

## Backfill

`trace-compactor backfill` packs the old archive, one JSON object per submission, into a local directory. It is a separate command with its own rules, because the old archive is in production and is only ever read:

```sh
trace-compactor backfill --out DIR [--also-packed DIR]... [--trace-data DIR] [--only PREFIX]
                         [--download-mb-per-second N] [--tp-schema N,N,...]
```

`scripts/backfill-days.sh` runs it one day received at a time, for a run that is left alone for hours.

- **What it writes:** `DIR/packs/backfill/YYYY/MM/DD/HH-<md5>.pack`, one pack per hour the submissions were received in, or several when an hour is larger than `--pack-input-mib`. Uploading `DIR/packs` is left to a sync tool, which **must upload each pack in one part**. A removal rewrites a pack only when its ETag is its MD5, and the ETag of an object uploaded in parts is not, so no submission could ever be removed from such a pack. rclone 1.75 uploads in parts from 256 MiB even with `--s3-upload-cutoff 5G`; add `--multi-thread-streams 0` (tried on 2026-11-27 with a 941 MiB pack).
- **What it reads:** a trace-data checkout (`--trace-data`), the old bucket (the `SOURCE_S3_*` variables), or both. With the bucket, the bucket says which submissions exist, and a local file saves the download only when its MD5 is the ETag the bucket lists. The bucket is reached through a type that can list and download and nothing else.
- **How much it asks of the old bucket:** one listing of what `--only` names, or of everything without it, and one download of each submission it packs. `--download-mb-per-second` holds all threads together to a limit; small objects still arrive in short bursts with pauses between them.
- **It packs what is in no pack under `DIR`, or under a directory named with `--also-packed`, yet**, so it can be stopped and run again, and a later run catches up. A submission that reaches the old archive after its hour was packed, because it was accepted days after it was received, gets a pack of its own for that hour.
- **Only hours that ended `--settle-hours` ago** are packed (48 by default), since a recent submission can still be rejected and deleted.
- **It deletes nothing.** A submission that is in a pack and no longer in the bucket was rejected since; it is reported, and stays packed until someone decides.
- A pack is written under another name and renamed, so a file named `.pack` is always whole. Anything else under `DIR/packs` stops the run.
- **What a run over days does not show.** A run with `--only` looks at nothing else, so it reports neither what was accepted late for another day nor what is gone from the bucket. Before the old bucket is retired, one run without `--only` has to find nothing left to pack, every pack's MD5 has to be the one in its name, and the checkout that the first packs were made from has to be compared with the bucket by checksum.

## Tests

```sh
cargo test -p trace-compactor
```

The tests of packing and of removal run against a bucket in memory that can be made to stop answering at any request. The two central ones stop a run at every request in turn, in both ways a request can fail, and then stop the run after it at every request too. They do not cover two processes at once, or a request that is carried out long after it was given up on.

The tests that need an S3 server are skipped unless asked for:

```sh
docker run --rm -p 127.0.0.1:19000:9000 -e RUSTFS_ACCESS_KEY=questie-test \
  -e RUSTFS_SECRET_KEY=questie-test-secret rustfs/rustfs
TRACE_COMPACTOR_TEST_S3=http://127.0.0.1:19000 cargo test -p trace-compactor -- --ignored
```

Three of them, named `provider_...`, check that a real provider answers the way the compactor reads it: an ETag that is the MD5, an upload refused when its body does not match its MD5, listings of more than one page, ranged downloads. With the `ARCHIVE_S3_*` variables set, they run against that bucket and touch only keys under `compactor-check/`:

```sh
TRACE_COMPACTOR_TEST_S3=provider cargo test -p trace-compactor provider -- --ignored
```

They passed against OVHcloud's London region on 2026-11-24.

## Not built

- **Alerts.** A failed run shows only in the service's log. The intake alerts on a removal request that stays unanswered.
- **A cache of pack indexes.** A run with an open removal request reads every pack's index: three requests a pack, in each of the two runs a removal takes.
