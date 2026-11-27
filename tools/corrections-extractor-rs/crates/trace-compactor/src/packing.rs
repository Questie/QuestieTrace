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
//!
//! A pack's key ends in the checksum the pack had when it was written. Two different packs
//! are therefore never written to one key, whatever a listing shows and whenever an upload
//! arrives, and an upload that is repeated stores the same bytes again.

use crate::bucket::{delete_unchanged, md5_hex, pack_intent_key, packs_prefix, submission_id, submission_name, Bucket, Listed, LOOSE};
use crate::Config;
use anyhow::{anyhow, bail, Context, Result};
use jiff::Timestamp;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::{Condvar, Mutex};
use trace_pack::pack::{build_frame, Frame, PackWriter, Scratch};
use trace_pack::record::Encoding;

/// What the packing step of one run did.
#[derive(Debug, Default, PartialEq)]
pub struct Packed {
    pub packs: Vec<PackReport>,
    /// One line for each settled loose object that was left where it is, with the reason,
    /// for someone to look at.
    pub left_loose: Vec<String>,
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

/// Packs every loose object that had settled at `now`, except the submissions with a removal
/// request, whose ids are `requested`. [`finish_interrupted`] must have run first.
pub fn pack_settled(bucket: &dyn Bucket, config: &Config, now: Timestamp, requested: &BTreeSet<String>) -> Result<Packed> {
    let budget = BuildBudget::new(config.build_input_bytes);
    let mut packed = Packed::default();
    for batch in select(bucket, config, now, requested)? {
        packed.packs.extend(pack(bucket, config, batch, now, &budget, &mut packed.left_loose)?);
    }
    Ok(packed)
}

// ---- Selecting ----

/// The settled loose objects, oldest first, in one group per pack.
///
/// The listing is taken once per run. A loose object that appears later is seen by the next
/// run.
///
/// A submission the intake wants removed is never packed: removing it from `loose/` is one
/// delete, where removing it from a pack rewrites the pack. One is still in `loose/` here
/// only when its request could not be answered.
fn select(bucket: &dyn Bucket, config: &Config, now: Timestamp, requested: &BTreeSet<String>) -> Result<Vec<Vec<Listed>>> {
    let settled_by = now.checked_sub(config.settle).context("the settle window reaches before the calendar")?;
    let mut settled: Vec<Listed> = bucket
        .list(LOOSE)?
        .into_iter()
        .filter(|loose| loose.stored_at <= settled_by)
        .filter(|loose| submission_name(&loose.key).is_some_and(|name| !requested.contains(&submission_id(name))))
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

// ---- Building ----

/// Limits how much loose JSON is being turned into frames at once. That is what a run's
/// memory follows: building one submission's frame takes about 22 times the size of its
/// file, measured on real traces, and every core builds one.
pub(crate) struct BuildBudget {
    limit: u64,
    in_use: Mutex<u64>,
    released: Condvar,
}

/// A share of the budget, given back when dropped.
pub(crate) struct Share<'a> {
    budget: &'a BuildBudget,
    bytes: u64,
}

impl BuildBudget {
    pub(crate) fn new(limit: u64) -> Self {
        BuildBudget {
            limit: limit.max(1),
            in_use: Mutex::new(0),
            released: Condvar::new(),
        }
    }

    /// Waits until `bytes` fit. An object larger than the whole budget takes all of it, so it
    /// is built alone; nothing can be done about what it needs by itself.
    pub(crate) fn take(&self, bytes: u64) -> Share<'_> {
        let bytes = bytes.clamp(1, self.limit);
        let mut in_use = self.in_use.lock().expect("no holder panics");
        while *in_use + bytes > self.limit {
            in_use = self.released.wait(in_use).expect("no holder panics");
        }
        *in_use += bytes;
        Share { budget: self, bytes }
    }
}

impl Drop for Share<'_> {
    fn drop(&mut self) {
        *self.budget.in_use.lock().expect("no holder panics") -= self.bytes;
        self.budget.released.notify_all();
    }
}

/// The frame of one loose object. The inner `Err` is a fault of this object alone: it is left
/// in `loose/` and the pack is made without it. The outer one is the bucket failing, which
/// stops the run.
fn build(bucket: &dyn Bucket, config: &Config, loose: &Listed, budget: &BuildBudget, scratch: &mut Scratch) -> Result<Result<Frame, String>> {
    let _share = budget.take(loose.size);
    let Some(file) = bucket.get(&loose.key)? else {
        return Ok(Err("gone since it was listed".into()));
    };
    // The object must be what the listing said: that checksum is what the intent records and
    // what the delete is later held to.
    if md5_hex(&file) != loose.etag {
        return Ok(Err(format!(
            "its ETag {} is not the MD5 of what was downloaded: it changed, or was not stored in one part",
            loose.etag
        )));
    }
    let name = submission_name(&loose.key).expect("selected by its name");
    Ok(build_frame(name, &file, &config.options, scratch))
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

/// Packs one batch. None when no object of it could be built.
fn pack(
    bucket: &dyn Bucket,
    config: &Config,
    mut batch: Vec<Listed>,
    now: Timestamp,
    budget: &BuildBudget,
    left_loose: &mut Vec<String>,
) -> Result<Option<PackReport>> {
    // A pack holds its submissions in the order they were received, which is the order of
    // their names.
    batch.sort_by(|a, b| a.key.cmp(&b.key));

    // Build, on every core.
    let built = batch
        .par_iter()
        .map_init(Scratch::new, |scratch, loose| build(bucket, config, loose, budget, scratch))
        .collect::<Result<Vec<_>>>()?;

    let mut report = PackReport {
        key: String::new(),
        submissions: 0,
        kept_whole: 0,
        value_fallbacks: Vec::new(),
        loose_bytes: 0,
        pack_bytes: 0,
    };
    let mut writer = PackWriter::new();
    let mut holds = Vec::new();
    for (loose, built) in batch.into_iter().zip(built) {
        match built {
            Err(reason) => left_loose.push(format!("{}: {reason}", loose.key)),
            Ok(frame) => {
                report.submissions += 1;
                report.kept_whole += frame.entry.exports.iter().any(|(encoding, _)| *encoding == Encoding::Raw) as usize;
                report.value_fallbacks.extend(frame.report.value_fallbacks.iter().cloned());
                report.loose_bytes += frame.report.file_bytes;
                writer.add(frame);
                holds.push(PackedLoose {
                    key: loose.key,
                    etag: loose.etag,
                });
            }
        }
    }
    if holds.is_empty() {
        return Ok(None);
    }
    let pack = writer
        .finish(config.options.level)
        .map_err(|message| anyhow!("the pack does not read back: {message}"))?;
    let pack_md5 = md5_hex(&pack);
    report.key = pack_key(&config.writer, now, &pack_md5);
    report.pack_bytes = pack.len() as u64;

    // Record, upload, confirm, delete.
    let intent = PackIntent {
        schema_version: PACK_INTENT_SCHEMA,
        pack: report.key.clone(),
        pack_md5,
        loose: holds,
    };
    let intent_key = pack_intent_key(&config.writer);
    bucket.put(&intent_key, &serde_json::to_vec_pretty(&intent)?)?;
    bucket.put(&intent.pack, &pack)?;
    complete(bucket, &intent_key, &intent)?;
    Ok(Some(report))
}

/// The key of a pack: `packs/<writer>/YYYY/MM/DD/HH-<md5>.pack`, from the hour of the run in
/// UTC and the checksum the pack had when it was written. A pack that is later rewritten
/// without a removed submission keeps its key, so the name is not a checksum to verify by.
fn pack_key(writer: &str, now: Timestamp, pack_md5: &str) -> String {
    format!("{}{}-{pack_md5}.pack", packs_prefix(writer), now.strftime("%Y/%m/%d/%H"))
}

/// Steps 3 and 4 after the upload: confirms the bucket holds the pack the intent names, then
/// deletes the loose objects it holds and the intent.
fn complete(bucket: &dyn Bucket, intent_key: &str, intent: &PackIntent) -> Result<()> {
    let stored = bucket
        .etag(&intent.pack)?
        .with_context(|| format!("{}: missing after its upload", intent.pack))?;
    if stored != intent.pack_md5 {
        bail!(
            "{}: the bucket holds an object with checksum {stored}, the pack that was built has {}",
            intent.pack,
            intent.pack_md5
        );
    }
    // Each only while it is still the object that was packed.
    intent
        .loose
        .par_iter()
        .try_for_each(|packed| delete_unchanged(bucket, &packed.key, &packed.etag))?;
    bucket.delete(intent_key)
}

// ---- Finishing an interrupted run ----

/// Finishes the pack an earlier run recorded and did not complete. An intent that cannot be
/// finished is an error, and nothing else runs past it.
pub fn finish_interrupted(bucket: &dyn Bucket, config: &Config) -> Result<()> {
    let intent_key = pack_intent_key(&config.writer);
    let Some(bytes) = bucket.get(&intent_key)? else {
        return Ok(());
    };
    let intent: PackIntent = serde_json::from_slice(&bytes).with_context(|| format!("{intent_key}: not a pack intent this version reads"))?;
    if intent.schema_version != PACK_INTENT_SCHEMA {
        bail!("{intent_key}: pack intent version {} is not supported", intent.schema_version);
    }

    // No pack under the recorded key. Deleting starts only after a pack is confirmed, so
    // every loose object is still there and is selected again. Should that upload still
    // arrive, it adds a pack whose submissions are in another one too, which readers accept.
    if bucket.etag(&intent.pack)?.is_none() {
        return bucket.delete(&intent_key);
    }
    complete(bucket, &intent_key, &intent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bucket::REMOVAL_REQUESTS;
    use crate::memory_bucket::{MemoryBucket, PutFault};
    use crate::test_support::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use trace_pack::pack::PackReader;

    /// The packing part of one whole run.
    fn run(bucket: &MemoryBucket, config: &Config, now: Timestamp) -> Result<Packed> {
        crate::run(bucket, config, now).map(|report| report.packed)
    }

    /// The names in a stored pack, in the pack's order.
    fn names_in(bucket: &MemoryBucket, pack_key: &str) -> Vec<String> {
        let pack = bucket.object(pack_key).unwrap();
        PackReader::open(&pack).unwrap().entries().iter().map(|entry| entry.name.clone()).collect()
    }

    #[test]
    fn settled_submissions_are_packed_and_deleted_and_younger_ones_wait() {
        let (bucket, files) = bucket_with(3, 49);
        let (young_name, young_file) = submission(30, "young");
        bucket.store(&format!("{LOOSE}{young_name}"), &young_file, hours_ago(47));
        // Folder markers, which some tools create, are not submissions.
        bucket.store(LOOSE, b"", hours_ago(49));
        bucket.store("loose/2026-11/", b"", hours_ago(49));

        let packed_now = run(&bucket, &config(u64::MAX), now()).unwrap();

        // One pack, named by the hour of the run and its own checksum, holding the three in
        // the order received.
        let pack_key = pack_keys(&bucket).pop().unwrap();
        let pack = bucket.object(&pack_key).unwrap();
        assert_eq!(pack_key, format!("packs/live/2026/11/24/22-{}.pack", md5_hex(&pack)));
        assert_eq!(names_in(&bucket, &pack_key), files.keys().cloned().collect::<Vec<_>>());
        assert_eq!(PackReader::open(&pack).unwrap().entries()[1].submission_id, id(1));
        assert_eq!(packed(&bucket, &files).len(), 3);

        // Left: what is not a submission, the one that has not settled, and the pack.
        assert_eq!(
            bucket.keys(""),
            ["loose/", "loose/2026-11/", "loose/2026-11/2026-11-08_183049Z_young.json", pack_key.as_str()]
        );
        assert_eq!(packed_now.packs.len(), 1);
        assert_eq!(packed_now.left_loose, [] as [String; 0]);
        assert_eq!((packed_now.packs[0].submissions, packed_now.packs[0].kept_whole), (3, 3));
        assert_eq!(
            (packed_now.packs[0].key.as_str(), packed_now.packs[0].pack_bytes),
            (pack_key.as_str(), pack.len() as u64)
        );
    }

    #[test]
    fn a_run_with_nothing_settled_writes_nothing() {
        let (bucket, _) = bucket_with(2, 47);
        assert_eq!(run(&bucket, &config(u64::MAX), now()).unwrap(), Packed::default());
        assert_eq!(bucket.keys("").len(), 2);
    }

    #[test]
    fn the_size_limit_splits_a_run_into_several_packs() {
        let (bucket, files) = bucket_with(5, 49);
        let one = files.values().next().unwrap().len() as u64;

        // Room for two submissions a pack: five make three packs.
        let packed_now = run(&bucket, &config(2 * one), now()).unwrap();

        assert_eq!(packed_now.packs.iter().map(|report| report.submissions).collect::<Vec<_>>(), [2, 2, 1]);
        // The first pack takes what the bucket stored first, which in this bucket is what was
        // received last.
        assert_eq!(names_in(&bucket, &packed_now.packs[0].key), files.keys().skip(3).cloned().collect::<Vec<_>>());
        assert_eq!(pack_keys(&bucket).len(), 3);
        assert_eq!(packed(&bucket, &files), files.keys().map(|name| (name.clone(), 1)).collect());
        assert_eq!(bucket.keys(LOOSE), [] as [&str; 0]);
    }

    #[test]
    fn a_submission_larger_than_the_limit_is_packed_alone() {
        let (bucket, files) = bucket_with(2, 49);
        let packed_now = run(&bucket, &config(1), now()).unwrap();
        assert_eq!(packed_now.packs.iter().map(|report| report.submissions).collect::<Vec<_>>(), [1, 1]);
        assert_eq!(packed(&bucket, &files).len(), 2);
    }

    /// The same submissions packed in another hour, or with one more, are another pack under
    /// another key: nothing a run writes can land on a pack that exists.
    #[test]
    fn packs_that_differ_never_share_a_key() {
        let key_of = |count: u32, hour: i64| {
            let (bucket, _) = bucket_with(count, 49 + hour);
            let at = now().checked_sub(jiff::SignedDuration::from_hours(hour)).unwrap();
            run(&bucket, &config(u64::MAX), at).unwrap().packs[0].key.clone()
        };
        assert_eq!(key_of(3, 0), key_of(3, 0));
        assert_ne!(key_of(3, 0), key_of(4, 0));
        assert!(key_of(3, 0).starts_with("packs/live/2026/11/24/22-") && key_of(3, 1).starts_with("packs/live/2026/11/24/21-"));
    }

    #[test]
    fn a_submission_whose_removal_request_cannot_be_answered_is_left_in_loose() {
        let (bucket, files) = bucket_with(3, 49);
        bucket.store(&format!("{REMOVAL_REQUESTS}{}", id(1)), b"not a request this version reads", now());

        run(&bucket, &config(u64::MAX), now()).unwrap();

        let found = packed(&bucket, &files);
        assert_eq!(found.len(), 2);
        assert!(found.keys().all(|name| !name.contains(&id(1))));
        assert_eq!(bucket.keys(LOOSE), [format!("{LOOSE}2026-11/2026-11-08_180149Z_{}.json", id(1))]);
    }

    /// The property the whole design rests on: wherever a run stops, and wherever the run
    /// after it stops too, every submission is still whole in `loose/` or in a stored pack,
    /// and the next run ends with each in a pack and nothing left over.
    #[test]
    fn a_run_stopped_at_any_request_loses_nothing_and_the_next_run_finishes() {
        let setup = || bucket_with(5, 49);
        let one = setup().1.values().next().unwrap().len() as u64;
        let two_packs = config(3 * one);
        let check = |bucket: &MemoryBucket, files: &BTreeMap<String, Vec<u8>>, when: &str| {
            let found = packed(bucket, files);
            for (name, file) in files {
                let loose = bucket.object(&format!("{LOOSE}{name}"));
                assert!(
                    loose.as_ref() == Some(file) || found.contains_key(name),
                    "{when}: {name} is in neither loose/ nor a pack"
                );
            }
        };
        let finished = |bucket: &MemoryBucket, files: &BTreeMap<String, Vec<u8>>, when: &str| {
            run(bucket, &two_packs, now()).unwrap_or_else(|error| panic!("{when}: the last run failed: {error:#}"));
            assert_eq!(packed(bucket, files).len(), 5, "{when}");
            assert_eq!(bucket.keys(LOOSE), [] as [&str; 0], "{when}");
            assert_eq!(bucket.keys("intents/"), [] as [&str; 0], "{when}");
        };

        let stopped_runs = stop_everywhere(setup, &two_packs, check, finished);
        // Two packs of five submissions take well over twenty requests.
        assert!(stopped_runs > 20, "only {stopped_runs} requests were failed");
    }

    #[test]
    fn a_loose_object_that_changed_after_it_was_packed_is_not_deleted() {
        let (bucket, files) = bucket_with(3, 49);
        stopped_after_the_upload(&bucket);
        let changed = format!("{LOOSE}{}", files.keys().next().unwrap());
        bucket.store(&changed, b"other bytes under the same key", now());

        let error = run(&bucket, &config(u64::MAX), now()).unwrap_err();

        assert!(format!("{error:#}").contains("changed since it was listed"), "{error:#}");
        assert_eq!(bucket.object(&changed).unwrap(), b"other bytes under the same key");
        // The intent stays, so every later run stops here too until someone has looked.
        assert_eq!(bucket.keys("intents/"), ["intents/live/pack.json"]);
    }

    #[test]
    fn an_intent_whose_pack_is_another_object_stops_the_run_and_deletes_nothing() {
        let (bucket, _) = bucket_with(3, 49);
        stopped_after_the_upload(&bucket);
        bucket.store(&pack_keys(&bucket)[0], b"not the pack that was built", now());

        let error = run(&bucket, &config(u64::MAX), now()).unwrap_err();

        assert!(format!("{error:#}").contains("the pack that was built has"), "{error:#}");
        assert_eq!(bucket.keys(LOOSE).len(), 3);
        assert_eq!(bucket.keys("intents/"), ["intents/live/pack.json"]);
    }

    /// The upload carries a checksum the provider enforces. Should a pack be stored wrongly
    /// all the same, or not at all, its loose objects must stay.
    #[test]
    fn a_pack_the_bucket_did_not_store_as_sent_keeps_its_loose_objects() {
        for (fault, message) in [
            (PutFault::Damaged, "the pack that was built has"),
            (PutFault::NotStored, "missing after its upload"),
        ] {
            let (bucket, _) = bucket_with(3, 49);
            bucket.fail_put("packs/", fault);

            let error = run(&bucket, &config(u64::MAX), now()).unwrap_err();

            assert!(format!("{error:#}").contains(message), "{fault:?}: {error:#}");
            assert_eq!(bucket.keys(LOOSE).len(), 3, "{fault:?}");
        }
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
            assert_eq!(pack_keys(&bucket), [] as [&str; 0]);
        }
    }

    /// One object that cannot be packed must not hold up the others, now or in later runs.
    #[test]
    fn a_loose_object_that_cannot_be_packed_is_left_and_reported_and_the_rest_is_packed() {
        let (bucket, mut files) = bucket_with(3, 49);
        let replaced = format!("{LOOSE}{}", files.keys().next().unwrap().clone());
        // It is replaced once it has been listed, so what is downloaded is not what was
        // listed. The same happens to an object that was not stored in one part.
        bucket.after_listing(LOOSE, &replaced, b"replaced after the listing");

        let packed_now = run(&bucket, &config(u64::MAX), now()).unwrap();

        assert_eq!(packed_now.left_loose.len(), 1);
        assert!(
            packed_now.left_loose[0].starts_with(&replaced) && packed_now.left_loose[0].contains("not the MD5"),
            "{:?}",
            packed_now.left_loose
        );
        assert_eq!(bucket.object(&replaced).unwrap(), b"replaced after the listing");
        assert_eq!(bucket.keys(LOOSE), [replaced.as_str()]);
        files.pop_first();
        assert_eq!(packed(&bucket, &files), files.keys().map(|name| (name.clone(), 1)).collect());
        assert_eq!(packed_now.packs[0].submissions, 2);
    }

    #[test]
    fn a_loose_object_that_is_gone_when_it_is_read_does_not_stop_the_others() {
        let (bucket, mut files) = bucket_with(3, 49);
        let gone = format!("{LOOSE}{}", files.keys().next().unwrap().clone());
        bucket.gone_after_listing(LOOSE, &gone);

        let packed_now = run(&bucket, &config(u64::MAX), now()).unwrap();

        assert_eq!(packed_now.left_loose, [format!("{gone}: gone since it was listed")]);
        files.pop_first();
        assert_eq!(packed(&bucket, &files), files.keys().map(|name| (name.clone(), 1)).collect());
        assert_eq!(bucket.keys(LOOSE), [] as [&str; 0]);
    }

    #[test]
    fn a_run_in_which_nothing_could_be_packed_writes_no_pack() {
        let (bucket, files) = bucket_with(1, 49);
        let only = format!("{LOOSE}{}", files.keys().next().unwrap());
        bucket.after_listing(LOOSE, &only, b"replaced after the listing");

        let packed_now = run(&bucket, &config(u64::MAX), now()).unwrap();

        assert_eq!((packed_now.packs.len(), packed_now.left_loose.len()), (0, 1));
        assert_eq!(bucket.keys(""), [only.as_str()]);
    }

    /// With room for one byte, every object is larger than the budget and is read and built
    /// alone, however many threads there are.
    #[test]
    fn the_build_budget_limits_how_much_a_run_reads_at_once() {
        let most_at_once = |build_input_bytes: u64| {
            let (bucket, _) = bucket_with(8, 49);
            bucket.slow_gets(Duration::from_millis(20));
            let config = Config {
                build_input_bytes,
                ..config(u64::MAX)
            };
            run(&bucket, &config, now()).unwrap();
            bucket.most_gets_at_once()
        };
        assert_eq!(most_at_once(1), 1);
        // Without the limit they overlap, so the count above is not one for another reason.
        if rayon::current_num_threads() > 1 {
            assert!(most_at_once(u64::MAX) > 1);
        }
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

        let packed_now = run(&bucket, &backfill, now()).unwrap();

        assert!(
            packed_now.packs[0].key.starts_with("packs/backfill/2026/11/24/22-"),
            "{}",
            packed_now.packs[0].key
        );
        assert_eq!(packed(&bucket, &files).len(), 2);
        assert_eq!(bucket.object("intents/live/pack.json").unwrap(), b"the live writer's");
    }

    /// The budget is what keeps a run's memory bounded: a share that does not fit must wait.
    #[test]
    fn the_build_budget_makes_what_does_not_fit_wait() {
        let budget = BuildBudget::new(10);
        let (first, second_taken) = (budget.take(6), AtomicBool::new(false));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let _second = budget.take(6);
                second_taken.store(true, Ordering::SeqCst);
            });
            std::thread::sleep(Duration::from_millis(100));
            assert!(!second_taken.load(Ordering::SeqCst), "6 more were given out of a budget of 10 with 6 in use");
            drop(first);
        });
        assert!(second_taken.load(Ordering::SeqCst));

        // More than the whole budget is given when nothing else is in use, and then holds
        // everything else back.
        let (alone, other_taken) = (budget.take(1000), AtomicBool::new(false));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let _other = budget.take(1);
                other_taken.store(true, Ordering::SeqCst);
            });
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                !other_taken.load(Ordering::SeqCst),
                "something was built beside an object larger than the budget"
            );
            drop(alone);
        });
    }
}
