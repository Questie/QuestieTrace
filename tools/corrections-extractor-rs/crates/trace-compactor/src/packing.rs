//! Packing: loose submissions that have settled become packs, and are then deleted.
//!
//! The order of one pack is what keeps every submission in the bucket at every moment:
//!
//! 1. Build the pack in memory. Every frame is read back against its loose object.
//! 2. Record the pack intent: the pack's key and checksum, and the loose objects it holds.
//! 3. Upload the pack in one request, then confirm what the bucket stored.
//! 4. Delete exactly the recorded loose objects, then the intent.
//!
//! A run that stops leaves at most one intent, which the next run finishes before anything
//! else. Nothing is deleted without a confirmed pack that holds it.

use crate::bucket::{md5_hex, pack_intent_key, packs_prefix, Bucket, Listed, LOOSE, REMOVAL_REQUESTS};
use anyhow::{anyhow, bail, Context, Result};
use jiff::{SignedDuration, Timestamp};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use trace_data::submission::submission_id_from_path;
use trace_pack::pack::{build_frame, PackOptions, PackWriter, Scratch};
use trace_pack::record::Encoding;

pub struct Config {
    /// Names this writer's packs and its intent: `live` for the compactor on the server.
    /// Only one process may run under a name at a time.
    pub writer: String,
    /// How long a loose object waits before it is packed, counted from when the bucket
    /// stored it.
    pub settle: SignedDuration,
    /// The most loose JSON one pack is built from, in bytes. A pack is built and uploaded
    /// from memory, so this bounds what a run needs. A single larger object gets a pack of
    /// its own.
    pub pack_input_bytes: u64,
    pub options: PackOptions,
}

/// What one pack was made of, for the run's log.
#[derive(Debug, PartialEq)]
pub struct PackReport {
    pub key: String,
    pub submissions: usize,
    /// Submissions stored as the file they were, because the extractors could not split them.
    pub kept_whole: usize,
    /// Exports that failed their check under the value encoding and were stored exact. Should
    /// be none: many are a sign of a fault in the encoder.
    pub value_fallbacks: Vec<String>,
    pub loose_bytes: u64,
    pub pack_bytes: u64,
}

/// One run: finishes what an earlier run left, then packs every loose object that had settled
/// at `now`.
pub fn run(bucket: &dyn Bucket, config: &Config, now: Timestamp) -> Result<Vec<PackReport>> {
    finish_interrupted(bucket, config)?;
    let mut reports = Vec::new();
    for batch in select(bucket, config, now)? {
        reports.push(pack(bucket, config, batch, now)?);
    }
    Ok(reports)
}

// ---- Selecting ----

/// The settled loose objects, oldest first, in one group per pack.
///
/// Both listings are taken once per run. A removal request or a loose object that appears
/// later is seen by the next run.
fn select(bucket: &dyn Bucket, config: &Config, now: Timestamp) -> Result<Vec<Vec<Listed>>> {
    // A submission the intake wants removed is never packed: removing it from `loose/` is one
    // delete, where removing it from a pack rewrites the pack.
    let requested: HashSet<String> = bucket
        .list(REMOVAL_REQUESTS)?
        .into_iter()
        .map(|request| request.key[REMOVAL_REQUESTS.len()..].to_owned())
        .collect();
    let settled_by = now.checked_sub(config.settle).context("the settle window reaches before the calendar")?;
    let mut settled: Vec<Listed> = bucket
        .list(LOOSE)?
        .into_iter()
        .filter(|loose| loose.stored_at <= settled_by)
        .filter(|loose| submission_name(&loose.key).is_some_and(|name| !requested.contains(&submission_id_from_path(Path::new(name)))))
        .collect();
    settled.sort_by(|a, b| (a.stored_at, &a.key).cmp(&(b.stored_at, &b.key)));

    let mut batches: Vec<Vec<Listed>> = Vec::new();
    let mut batch_bytes = 0;
    for loose in settled {
        if batches.is_empty() || batch_bytes + loose.size > config.pack_input_bytes {
            batches.push(Vec::new());
            batch_bytes = 0;
        }
        batch_bytes += loose.size;
        batches.last_mut().expect("a batch was just started").push(loose);
    }
    Ok(batches)
}

/// The submission name in a loose key: what follows `loose/`. None for a key that names no
/// object of its own, such as the folder marker some tools create.
fn submission_name(key: &str) -> Option<&str> {
    key.strip_prefix(LOOSE).filter(|name| !name.is_empty() && !name.ends_with('/'))
}

// ---- Packing ----

/// The record of a pack that is being stored: step 2 of the module's list. It exists from
/// before the upload until the last loose object it names is deleted.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct PackIntent {
    schema_version: u32,
    pack: String,
    pack_md5: String,
    loose: Vec<PackedLoose>,
}

/// A loose object a pack holds, as it was when it was packed.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
struct PackedLoose {
    key: String,
    etag: String,
}

const PACK_INTENT_SCHEMA: u32 = 1;

fn pack(bucket: &dyn Bucket, config: &Config, mut batch: Vec<Listed>, now: Timestamp) -> Result<PackReport> {
    // A pack holds its submissions in the order they were received, which is the order of
    // their names.
    batch.sort_by(|a, b| a.key.cmp(&b.key));

    // Build, on every core. A loose object must still be what the listing said: its checksum
    // is what the intent records and what the delete is later held to.
    let frames = batch
        .par_iter()
        .map_init(Scratch::new, |scratch, loose| {
            let file = bucket.get(&loose.key)?.with_context(|| format!("{}: gone since it was listed", loose.key))?;
            if md5_hex(&file) != loose.etag {
                bail!("{}: what was downloaded is not what was listed", loose.key);
            }
            let name = submission_name(&loose.key).expect("selected by its name");
            build_frame(name, &file, &config.options, scratch).map_err(|message| anyhow!("{}: {message}", loose.key))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut report = PackReport {
        key: next_pack_key(bucket, &config.writer, now)?,
        submissions: frames.len(),
        kept_whole: 0,
        value_fallbacks: Vec::new(),
        loose_bytes: 0,
        pack_bytes: 0,
    };
    let mut writer = PackWriter::new();
    for frame in frames {
        report.kept_whole += frame.entry.exports.iter().any(|(encoding, _)| *encoding == Encoding::Raw) as usize;
        report.value_fallbacks.extend(frame.report.value_fallbacks.iter().cloned());
        report.loose_bytes += frame.report.file_bytes;
        writer.add(frame);
    }
    let pack = writer.finish(config.options.level).map_err(|message| anyhow!("{}: {message}", report.key))?;
    report.pack_bytes = pack.len() as u64;

    // Record, upload, confirm, delete.
    let intent = PackIntent {
        schema_version: PACK_INTENT_SCHEMA,
        pack: report.key.clone(),
        pack_md5: md5_hex(&pack),
        loose: batch
            .into_iter()
            .map(|loose| PackedLoose {
                key: loose.key,
                etag: loose.etag,
            })
            .collect(),
    };
    let intent_key = pack_intent_key(&config.writer);
    bucket.put(&intent_key, &serde_json::to_vec_pretty(&intent)?)?;
    bucket.put(&intent.pack, &pack)?;
    complete(bucket, &intent_key, &intent)?;
    Ok(report)
}

/// The key of this writer's next pack: `packs/<writer>/YYYY/MM/DD/HH-n.pack`, from the hour
/// of the run in UTC and the first `n` no pack of that hour has. A pack is never replaced.
fn next_pack_key(bucket: &dyn Bucket, writer: &str, now: Timestamp) -> Result<String> {
    let hour = format!("{}{}-", packs_prefix(writer), now.strftime("%Y/%m/%d/%H"));
    let taken = bucket
        .list(&hour)?
        .iter()
        .filter_map(|pack| pack.key[hour.len()..].strip_suffix(".pack")?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    Ok(format!("{hour}{}.pack", taken + 1))
}

/// Steps 3 and 4 after the upload: confirms the bucket holds the pack the intent names, then
/// deletes the loose objects it holds and the intent.
fn complete(bucket: &dyn Bucket, intent_key: &str, intent: &PackIntent) -> Result<()> {
    let stored = bucket
        .stat(&intent.pack)?
        .with_context(|| format!("{}: missing after its upload", intent.pack))?;
    if stored.etag != intent.pack_md5 {
        bail!(
            "{}: the bucket holds an object with checksum {}, the pack that was built has {}",
            intent.pack,
            stored.etag,
            intent.pack_md5
        );
    }
    intent.loose.par_iter().try_for_each(|packed| delete_packed(bucket, packed))?;
    bucket.delete(intent_key)
}

/// Deletes a loose object only while it is still the object that was packed. An absent one
/// was deleted by the run this one finishes.
fn delete_packed(bucket: &dyn Bucket, packed: &PackedLoose) -> Result<()> {
    match bucket.stat(&packed.key)? {
        None => Ok(()),
        Some(stored) if stored.etag == packed.etag => bucket.delete(&packed.key),
        Some(_) => bail!("{}: changed since it was packed, so it is not deleted", packed.key),
    }
}

// ---- Finishing an interrupted run ----

/// Finishes the pack an earlier run recorded and did not complete. An intent that cannot be
/// finished is an error, and nothing else runs past it.
fn finish_interrupted(bucket: &dyn Bucket, config: &Config) -> Result<()> {
    let intent_key = pack_intent_key(&config.writer);
    let Some(bytes) = bucket.get(&intent_key)? else {
        return Ok(());
    };
    let intent: PackIntent = serde_json::from_slice(&bytes).with_context(|| format!("{intent_key}: not a pack intent this version reads"))?;
    if intent.schema_version != PACK_INTENT_SCHEMA {
        bail!("{intent_key}: pack intent version {} is not supported", intent.schema_version);
    }

    // No pack under the recorded key: the upload did not happen, and deleting starts only
    // after a confirmed upload. Every loose object is still there and is selected again.
    if bucket.stat(&intent.pack)?.is_none() {
        return bucket.delete(&intent_key);
    }
    complete(bucket, &intent_key, &intent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_bucket::{Fault, MemoryBucket};
    use std::collections::BTreeMap;
    use trace_pack::pack::PackReader;
    use trace_pack::record::Record;

    const NOW: &str = "2026-11-24T22:30:00Z";

    fn now() -> Timestamp {
        NOW.parse().unwrap()
    }

    fn hours_ago(hours: i64) -> Timestamp {
        now().checked_sub(SignedDuration::from_hours(hours)).unwrap()
    }

    fn config(pack_input_bytes: u64) -> Config {
        Config {
            writer: "live".into(),
            settle: SignedDuration::from_hours(48),
            pack_input_bytes,
            options: PackOptions::default(),
        }
    }

    /// A submission as the intake stores it. Its export string holds no export, so a pack
    /// keeps the file whole and a test can compare what it reads back byte for byte.
    fn submission(minute: u32, id: &str) -> (String, Vec<u8>) {
        let name = format!("2026-11/2026-11-08_18{minute:02}49Z_{id}.json");
        let file = format!(
            r#"{{"schema_version":2,"id":"{id}","contributor_id":"contributor-{id}","received_at":"2026-11-08T18:{minute:02}:49Z","bytes":9,"export_string":"not one {id}"}}"#
        );
        (name, file.into_bytes())
    }

    /// A bucket holding `count` loose submissions stored `stored_hours_ago`, and their files
    /// by name.
    fn bucket_with(count: u32, stored_hours_ago: i64) -> (MemoryBucket, BTreeMap<String, Vec<u8>>) {
        let bucket = MemoryBucket::new(now());
        let mut files = BTreeMap::new();
        for index in 0..count {
            let (name, file) = submission(index, &format!("{index:032x}"));
            // The later a submission was received, the earlier the bucket stored it: the order
            // in a pack must come from the names, not from the bucket's clock.
            let stored_at = hours_ago(stored_hours_ago).checked_sub(SignedDuration::from_secs(index.into())).unwrap();
            bucket.store(&format!("{LOOSE}{name}"), &file, stored_at);
            files.insert(name, file);
        }
        (bucket, files)
    }

    /// In how many stored packs each submission is. Every pack in the bucket must open, and
    /// every record in it must be the file it was made from.
    fn packed(bucket: &MemoryBucket, files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, usize> {
        let mut found = BTreeMap::new();
        for key in bucket.keys("packs/") {
            let pack = bucket.object(&key).unwrap();
            let reader = PackReader::open(&pack).unwrap_or_else(|message| panic!("{key} does not open: {message}"));
            for (index, entry) in reader.entries().iter().enumerate() {
                match reader.record(index).unwrap() {
                    Record::File { name, bytes } => assert_eq!(Some(&bytes), files.get(&name), "{key} holds {name} with other bytes"),
                    Record::Exports { name, .. } => panic!("{name} was not kept whole"),
                }
                *found.entry(entry.name.clone()).or_default() += 1;
            }
        }
        found
    }

    #[test]
    fn settled_submissions_are_packed_and_deleted_and_younger_ones_wait() {
        let (bucket, files) = bucket_with(3, 49);
        let (young_name, young_file) = submission(30, "young");
        bucket.store(&format!("{LOOSE}{young_name}"), &young_file, hours_ago(47));
        // Folder markers, which some tools create, are not submissions.
        bucket.store(LOOSE, b"", hours_ago(49));
        bucket.store("loose/2026-11/", b"", hours_ago(49));

        let reports = run(&bucket, &config(u64::MAX), now()).unwrap();

        // One pack, under the hour of the run, holding the three in the order received.
        assert_eq!(bucket.keys("packs/"), ["packs/live/2026/11/24/22-1.pack"]);
        let pack = bucket.object("packs/live/2026/11/24/22-1.pack").unwrap();
        let reader = PackReader::open(&pack).unwrap();
        let names: Vec<&str> = reader.entries().iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, files.keys().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(reader.entries()[1].submission_id, format!("{:032x}", 1));
        assert_eq!(packed(&bucket, &files).len(), 3);

        // Left: what is not a submission, the one that has not settled, and the pack.
        assert_eq!(
            bucket.keys(""),
            [
                "loose/",
                "loose/2026-11/",
                "loose/2026-11/2026-11-08_183049Z_young.json",
                "packs/live/2026/11/24/22-1.pack"
            ]
        );
        assert_eq!(reports.len(), 1);
        assert_eq!((reports[0].submissions, reports[0].kept_whole), (3, 3));
        assert_eq!(reports[0].pack_bytes, pack.len() as u64);
    }

    #[test]
    fn a_run_with_nothing_settled_writes_nothing() {
        let (bucket, _) = bucket_with(2, 47);
        assert_eq!(run(&bucket, &config(u64::MAX), now()).unwrap(), []);
        assert_eq!(bucket.keys("").len(), 2);
    }

    #[test]
    fn the_size_limit_splits_a_run_into_packs_that_never_share_a_key() {
        let (bucket, files) = bucket_with(5, 49);
        let one = files.values().next().unwrap().len() as u64;
        // An earlier run of the same hour left a pack; its key is taken.
        bucket.store("packs/live/2026/11/24/22-1.pack", b"an earlier pack", now());

        // Room for two submissions a pack: five make three packs.
        let reports = run(&bucket, &config(2 * one), now()).unwrap();

        assert_eq!(bucket.object("packs/live/2026/11/24/22-1.pack").unwrap(), b"an earlier pack");
        let keys: Vec<&str> = reports.iter().map(|report| report.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "packs/live/2026/11/24/22-2.pack",
                "packs/live/2026/11/24/22-3.pack",
                "packs/live/2026/11/24/22-4.pack"
            ]
        );
        assert_eq!(reports.iter().map(|report| report.submissions).collect::<Vec<_>>(), [2, 2, 1]);
        // The first pack takes what the bucket stored first, which in this bucket is what was
        // received last.
        let first = bucket.object("packs/live/2026/11/24/22-2.pack").unwrap();
        let first: Vec<String> = PackReader::open(&first).unwrap().entries().iter().map(|entry| entry.name.clone()).collect();
        assert_eq!(first, files.keys().skip(3).cloned().collect::<Vec<_>>());
        bucket.remove("packs/live/2026/11/24/22-1.pack");
        assert!(packed(&bucket, &files).values().all(|packs| *packs == 1));
        assert_eq!(packed(&bucket, &files).len(), 5);
        assert_eq!(bucket.keys(LOOSE), [] as [&str; 0]);
    }

    #[test]
    fn a_submission_larger_than_the_limit_is_packed_alone() {
        let (bucket, files) = bucket_with(2, 49);
        let reports = run(&bucket, &config(1), now()).unwrap();
        assert_eq!(reports.iter().map(|report| report.submissions).collect::<Vec<_>>(), [1, 1]);
        assert_eq!(packed(&bucket, &files).len(), 2);
    }

    #[test]
    fn a_submission_with_a_removal_request_is_left_in_loose() {
        let (bucket, files) = bucket_with(3, 49);
        let requested = format!("{:032x}", 1);
        bucket.store(&format!("{REMOVAL_REQUESTS}{requested}"), b"{}", now());

        run(&bucket, &config(u64::MAX), now()).unwrap();

        let found = packed(&bucket, &files);
        assert_eq!(found.len(), 2);
        assert!(found.keys().all(|name| !name.contains(&requested)));
        assert_eq!(bucket.keys(LOOSE), [format!("{LOOSE}2026-11/2026-11-08_180149Z_{requested}.json")]);
    }

    /// The property the whole design rests on: wherever a run stops, every submission is
    /// still whole in `loose/` or in a stored pack, and the next run ends with each in
    /// exactly one pack and nothing left over.
    #[test]
    fn a_run_stopped_at_any_request_loses_nothing_and_the_next_run_finishes() {
        for fault in [Fault::Refused, Fault::AnswerLost] {
            let mut stopped_runs = 0;
            for call in 0.. {
                let (bucket, files) = bucket_with(5, 49);
                let one = files.values().next().unwrap().len() as u64;
                bucket.fail_call(call, fault);
                let outcome = run(&bucket, &config(3 * one), now());
                let stopped = bucket.restore();
                assert_eq!(outcome.is_err(), stopped, "call {call}, {fault:?}: {outcome:?}");

                let found = packed(&bucket, &files);
                for (name, file) in &files {
                    let loose = bucket.object(&format!("{LOOSE}{name}"));
                    assert!(
                        loose.as_ref() == Some(file) || found.contains_key(name),
                        "call {call}, {fault:?}: {name} is in neither loose/ nor a pack"
                    );
                }
                if !stopped {
                    break;
                }
                stopped_runs += 1;

                run(&bucket, &config(3 * one), now()).unwrap_or_else(|error| panic!("call {call}, {fault:?}: the next run failed: {error:#}"));
                let found = packed(&bucket, &files);
                assert!(found.len() == 5 && found.values().all(|packs| *packs == 1), "call {call}, {fault:?}: {found:?}");
                assert_eq!(bucket.keys(LOOSE), [] as [&str; 0], "call {call}, {fault:?}");
                assert_eq!(bucket.keys("intents/"), [] as [&str; 0], "call {call}, {fault:?}");
            }
            // Two packs of five submissions take well over twenty requests.
            assert!(stopped_runs > 20, "only {stopped_runs} requests were failed");
        }
    }

    /// Stops a run right after its pack is stored, so the intent and every loose object are
    /// left for the next run.
    fn stopped_after_the_upload(bucket: &MemoryBucket) {
        let pack_put = (0..)
            .find(|call| {
                let (probe, _) = bucket_with(3, 49);
                probe.fail_call(*call, Fault::AnswerLost);
                let _ = run(&probe, &config(u64::MAX), now());
                probe.restore() && !probe.keys("packs/").is_empty()
            })
            .unwrap();
        bucket.fail_call(pack_put, Fault::AnswerLost);
        run(bucket, &config(u64::MAX), now()).unwrap_err();
        assert!(bucket.restore());
        assert_eq!(bucket.keys(LOOSE).len(), 3);
    }

    #[test]
    fn a_loose_object_that_changed_after_it_was_packed_is_not_deleted() {
        let (bucket, files) = bucket_with(3, 49);
        stopped_after_the_upload(&bucket);
        let changed = format!("{LOOSE}{}", files.keys().next().unwrap());
        bucket.store(&changed, b"other bytes under the same key", now());

        let error = run(&bucket, &config(u64::MAX), now()).unwrap_err();

        assert!(format!("{error:#}").contains("changed since it was packed"), "{error:#}");
        assert_eq!(bucket.object(&changed).unwrap(), b"other bytes under the same key");
        // The intent stays, so every later run stops here too until someone has looked.
        assert_eq!(bucket.keys("intents/"), ["intents/live/pack.json"]);
    }

    #[test]
    fn an_intent_whose_pack_is_another_object_stops_the_run_and_deletes_nothing() {
        let (bucket, _) = bucket_with(3, 49);
        stopped_after_the_upload(&bucket);
        bucket.store("packs/live/2026/11/24/22-1.pack", b"not the pack that was built", now());

        let error = run(&bucket, &config(u64::MAX), now()).unwrap_err();

        assert!(format!("{error:#}").contains("the pack that was built has"), "{error:#}");
        assert_eq!(bucket.keys(LOOSE).len(), 3);
        assert_eq!(bucket.keys("intents/"), ["intents/live/pack.json"]);
    }

    #[test]
    fn an_intent_this_version_cannot_read_stops_the_run() {
        let (bucket, _) = bucket_with(3, 49);
        for unreadable in [
            &b"not json"[..],
            br#"{"schema_version":2,"pack":"packs/live/x.pack","pack_md5":"00","loose":[]}"#,
            br#"{"schema_version":1,"pack":"packs/live/x.pack","pack_md5":"00","loose":[],"also_delete":[]}"#,
        ] {
            bucket.store("intents/live/pack.json", unreadable, now());
            run(&bucket, &config(u64::MAX), now()).unwrap_err();
            assert_eq!(bucket.keys(LOOSE).len(), 3);
            assert_eq!(bucket.keys("packs/"), [] as [&str; 0]);
        }
    }

    #[test]
    fn a_loose_object_replaced_while_the_run_reads_it_stops_the_pack() {
        let (bucket, files) = bucket_with(2, 49);
        let replaced = format!("{LOOSE}{}", files.keys().next().unwrap());
        // The listing is the run's second request to the bucket's loose prefix; replace the
        // object once it has been listed.
        bucket.after_listing(LOOSE, &replaced, b"replaced after the listing");

        let error = run(&bucket, &config(u64::MAX), now()).unwrap_err();

        assert!(format!("{error:#}").contains("not what was listed"), "{error:#}");
        assert_eq!(bucket.keys("packs/"), [] as [&str; 0]);
        assert_eq!(bucket.keys(LOOSE).len(), 2);
    }

    #[test]
    fn another_writer_has_its_own_packs_and_intent() {
        let (bucket, files) = bucket_with(2, 49);
        let backfill = Config {
            writer: "backfill".into(),
            ..config(u64::MAX)
        };
        // The server's compactor stopped mid-pack; that is not the backfill's to finish.
        bucket.store("intents/live/pack.json", b"the live writer's", now());

        let reports = run(&bucket, &backfill, now()).unwrap();

        assert_eq!(reports[0].key, "packs/backfill/2026/11/24/22-1.pack");
        assert_eq!(packed(&bucket, &files).len(), 2);
        assert_eq!(bucket.object("intents/live/pack.json").unwrap(), b"the live writer's");
    }
}
