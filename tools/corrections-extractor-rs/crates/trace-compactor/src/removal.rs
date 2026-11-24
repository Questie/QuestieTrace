//! Removals: a submission the intake wants gone leaves `loose/` and every pack that holds it.
//!
//! The intake writes `removals/requested/<id>` when a rejection of an accepted submission has
//! become final, and purges its own receipt of the submission once `removals/done/<id>` is
//! listed. For each request without that answer, a run does one of two things:
//!
//! - **The submission is found**, in `loose/` or in any pack's index. Record that the removal
//!   starts, `removals/started/<id>`, naming what was found. Delete the loose object. Rewrite
//!   each pack that holds the submission without it. Write no answer.
//! - **The submission is found nowhere, and its removal was started.** Write the answer,
//!   `removals/done/<id>`.
//!
//! So the answer always comes from a run that looked everywhere afresh and found nothing,
//! never from the run that deleted: a removal takes two runs. Finding the submission nowhere
//! without a started removal is not an answer either: that is also what a mistake in the
//! search would look like.

use crate::bucket::{
    delete_unchanged, md5_hex, submission_id, submission_name, Bucket, Listed, LOOSE, PACKS, REMOVAL_INTENTS, REMOVAL_RECEIPTS, REMOVAL_REQUESTS,
};
use anyhow::{anyhow, bail, Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use trace_pack::pack;

/// What the removal step of one run did.
#[derive(Debug, Default, PartialEq)]
pub struct Removals {
    /// Submissions this run deleted from `loose/` or took out of packs. The next run answers
    /// their requests.
    pub removed: Vec<Removed>,
    /// The ids whose requests this run answered.
    pub answered: Vec<String>,
    /// One line for each request that could not be worked on, with the reason. Each is left
    /// open for someone to look at.
    pub unanswered: Vec<String>,
    /// Why the step ended before it had gone through every request, when it did. The rest of
    /// the run goes on.
    pub stopped: Option<String>,
}

/// One submission whose stored copies a run removed.
#[derive(Debug, PartialEq)]
pub struct Removed {
    pub id: String,
    pub loose_deleted: usize,
    pub packs_rewritten: Vec<String>,
}

/// The ids of every submission with a removal request, answered or not.
pub fn requested(bucket: &dyn Bucket) -> Result<BTreeSet<String>> {
    Ok(ids_under(bucket.list(REMOVAL_REQUESTS)?, REMOVAL_REQUESTS))
}

/// The ids the keys under `prefix` name. A key that names a folder, which some tools create
/// as a marker, is not one.
fn ids_under(listed: Vec<Listed>, prefix: &str) -> BTreeSet<String> {
    listed
        .into_iter()
        .map(|object| object.key[prefix.len()..].to_owned())
        .filter(|id| !id.is_empty() && !id.contains('/'))
        .collect()
}

/// Works on every request that has no answer yet, adding to `removals` as it goes. `level`
/// compresses the index of a rewritten pack.
///
/// No pack may be mid-upload: the caller finishes an interrupted pack first. And nothing else
/// may write packs meanwhile, under any writer name: a rewrite replaces a pack with what this
/// run read of it.
pub fn answer(bucket: &dyn Bucket, requested: &BTreeSet<String>, level: i32, removals: &mut Removals) -> Result<()> {
    let answered = ids_under(bucket.list(REMOVAL_RECEIPTS)?, REMOVAL_RECEIPTS);
    let open: BTreeSet<&str> = requested.difference(&answered).map(String::as_str).collect();
    if open.is_empty() {
        return Ok(());
    }

    // Where each requested submission is. Read once for all requests; a pack that is rewritten
    // below is brought up to date here.
    let loose = bucket.list(LOOSE)?;
    let mut packs = packs_holding(bucket, &open)?;

    for id in open {
        match remove(bucket, id, &loose, &mut packs, level)? {
            Outcome::Removed(removed) => removals.removed.push(removed),
            Outcome::Answered => removals.answered.push(id.to_owned()),
            Outcome::Open(reason) => removals.unanswered.push(format!("{id}: {reason}")),
        }
    }
    Ok(())
}

/// A pack in the bucket, and which of the requested submissions its index lists.
struct StoredPack {
    key: String,
    etag: String,
    holds: BTreeSet<String>,
}

/// Every pack of every writer, with the submissions of `wanted` it holds. An index that
/// cannot be read is an error: without it nothing can be said about where a submission is
/// not.
///
/// Only the wanted ids are kept. The indexes together list every submission of the archive.
fn packs_holding(bucket: &dyn Bucket, wanted: &BTreeSet<&str>) -> Result<Vec<StoredPack>> {
    bucket
        .list(PACKS)?
        .into_par_iter()
        // Folder markers, which some tools create. Every other object here must be a pack.
        .filter(|listed| !listed.key.ends_with('/'))
        .map(|listed| {
            let fetch = |range| match bucket.get_range(&listed.key, range) {
                Ok(Some(bytes)) => Ok(bytes),
                Ok(None) => Err("gone since it was listed".to_owned()),
                Err(error) => Err(format!("{error:#}")),
            };
            let entries = pack::read_index(listed.size, fetch).map_err(|message| anyhow!("{}: {message}", listed.key))?;
            let holds = entries
                .iter()
                .map(|entry| submission_id(&entry.name))
                .filter(|id| wanted.contains(id.as_str()))
                .collect();
            Ok(StoredPack {
                key: listed.key,
                etag: listed.etag,
                holds,
            })
        })
        .collect()
}

/// What the intake wrote. Only the id is used; the request may carry more.
#[derive(Deserialize)]
struct Request {
    schema_version: u32,
    id: String,
}

/// The record that a removal was started. Written once and kept for good.
#[derive(Serialize)]
struct RemovalIntent<'a> {
    schema_version: u32,
    id: &'a str,
    loose: Vec<Found<'a>>,
    packs: Vec<Found<'a>>,
}

/// Where the submission was when its removal started.
#[derive(Serialize)]
struct Found<'a> {
    key: &'a str,
    etag: &'a str,
}

#[derive(Serialize)]
struct Receipt<'a> {
    schema_version: u32,
    id: &'a str,
}

/// What a run did about one request.
enum Outcome {
    /// Its stored copies were removed. The request is answered by a later run.
    Removed(Removed),
    /// Nothing of it was found and its removal had been started: the answer is written.
    Answered,
    /// It could not be worked on, for this reason.
    Open(String),
}

/// Works on one request. An `Err` is the bucket failing, or holding something other than
/// what was listed.
fn remove(bucket: &dyn Bucket, id: &str, loose: &[Listed], packs: &mut [StoredPack], level: i32) -> Result<Outcome> {
    // The request must be one this version understands, and about the id its key names.
    let Some(request) = bucket.get(&format!("{REMOVAL_REQUESTS}{id}"))? else {
        return Ok(Outcome::Open("its request is gone since it was listed".into()));
    };
    match serde_json::from_slice::<Request>(&request) {
        Ok(request) if request.schema_version == 1 && request.id == id => {}
        _ => return Ok(Outcome::Open("its request is not one this version reads, or names another submission".into())),
    }

    // Every occurrence, by the id in its name.
    let loose_copies: Vec<&Listed> = loose
        .iter()
        .filter(|object| submission_name(&object.key).is_some_and(|name| submission_id(name) == id))
        .collect();
    let holding: Vec<usize> = (0..packs.len()).filter(|&index| packs[index].holds.contains(id)).collect();
    let intent_key = format!("{REMOVAL_INTENTS}{id}");
    let started = bucket.etag(&intent_key)?.is_some();

    if loose_copies.is_empty() && holding.is_empty() {
        if !started {
            return Ok(Outcome::Open("found in neither loose/ nor any pack, and no removal of it was started".into()));
        }
        // From here on the intake forgets the submission.
        let receipt = Receipt { schema_version: 1, id };
        bucket.put(&format!("{REMOVAL_RECEIPTS}{id}"), &serde_json::to_vec_pretty(&receipt)?)?;
        return Ok(Outcome::Answered);
    }

    if !started {
        let found = |key, etag| Found { key, etag };
        let intent = RemovalIntent {
            schema_version: 1,
            id,
            loose: loose_copies.iter().map(|object| found(&object.key, &object.etag)).collect(),
            packs: holding.iter().map(|&index| found(&packs[index].key, &packs[index].etag)).collect(),
        };
        bucket.put(&intent_key, &serde_json::to_vec_pretty(&intent)?)?;
    }
    for object in &loose_copies {
        delete_unchanged(bucket, &object.key, &object.etag)?;
    }
    for &index in &holding {
        rewrite_without(bucket, &mut packs[index], id, level)?;
    }
    Ok(Outcome::Removed(Removed {
        id: id.to_owned(),
        loose_deleted: loose_copies.len(),
        packs_rewritten: holding.iter().map(|&index| packs[index].key.clone()).collect(),
    }))
}

/// Replaces a pack with itself minus one submission, under the same key.
///
/// The upload replaces the only copy of every other submission in the pack. The pack that is
/// downloaded must therefore be the one that was listed, and [`pack::without`] compares every
/// frame it keeps with that pack before anything is uploaded.
fn rewrite_without(bucket: &dyn Bucket, stored: &mut StoredPack, id: &str, level: i32) -> Result<()> {
    let old = bucket.get(&stored.key)?.with_context(|| format!("{}: gone since it was listed", stored.key))?;
    if md5_hex(&old) != stored.etag {
        bail!("{}: what was downloaded is not the pack that was listed", stored.key);
    }
    let rewritten = pack::without(&old, level, |entry| submission_id(&entry.name) == id).map_err(|message| anyhow!("{}: {message}", stored.key))?;
    drop(old);

    bucket.put(&stored.key, &rewritten)?;
    let etag = md5_hex(&rewritten);
    if bucket.etag(&stored.key)?.as_deref() != Some(etag.as_str()) {
        bail!("{}: the bucket does not hold the rewritten pack", stored.key);
    }
    // The next request may be for another submission of the same pack.
    stored.etag = etag;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_bucket::{MemoryBucket, PutFault};
    use crate::test_support::*;
    use crate::{run, RunReport};
    use std::collections::BTreeMap;

    /// A bucket whose `count` submissions are all in one pack, their files by name, and the
    /// pack's key.
    fn bucket_with_a_pack(count: u32) -> (MemoryBucket, BTreeMap<String, Vec<u8>>, String) {
        let (bucket, files) = bucket_with(count, 49);
        run(&bucket, &config(u64::MAX), now()).unwrap();
        let pack = pack_keys(&bucket).pop().unwrap();
        assert_eq!(bucket.keys(""), [pack.as_str()]);
        (bucket, files, pack)
    }

    /// The request the intake writes for a submission.
    fn request_for(bucket: &MemoryBucket, id: &str, name: &str) {
        let body = format!("{{\n  \"schema_version\": 1,\n  \"id\": \"{id}\",\n  \"name\": \"{name}\"\n}}\n");
        bucket.store(&format!("{REMOVAL_REQUESTS}{id}"), body.as_bytes(), now());
    }

    /// The request for the submission `bucket_with` stores at this position.
    fn request(bucket: &MemoryBucket, index: u32) {
        request_for(bucket, &id(index), &name(index));
    }

    fn has_receipt(bucket: &MemoryBucket, index: u32) -> bool {
        bucket.object(&format!("{REMOVAL_RECEIPTS}{}", id(index))).is_some()
    }

    /// The name of the submission `bucket_with` stores at this position.
    fn name(index: u32) -> String {
        submission(index, &id(index)).0
    }

    fn run_once(bucket: &MemoryBucket) -> RunReport {
        run(bucket, &config(u64::MAX), now()).unwrap()
    }

    const NOTHING: [String; 0] = [];

    #[test]
    fn a_requested_submission_still_in_loose_is_deleted_and_the_next_run_answers() {
        let (bucket, files) = bucket_with(3, 47);
        request(&bucket, 1);
        let request_before = bucket.object(&format!("{REMOVAL_REQUESTS}{}", id(1))).unwrap();

        let first = run_once(&bucket);

        assert_eq!(bucket.keys(LOOSE), [format!("{LOOSE}{}", name(0)), format!("{LOOSE}{}", name(2))]);
        assert_eq!(bucket.keys(REMOVAL_INTENTS), [format!("{REMOVAL_INTENTS}{}", id(1))]);
        assert_eq!((first.removals.removed.len(), first.removals.removed[0].loose_deleted), (1, 1));
        // The run that deleted does not answer.
        assert!(!has_receipt(&bucket, 1));
        assert_eq!(first.removals.answered, NOTHING);

        let second = run_once(&bucket);

        assert!(has_receipt(&bucket, 1));
        assert_eq!(second.removals.answered, [id(1)]);
        assert_eq!(second.removals.removed, []);
        // The request is the intake's: it is neither changed nor removed.
        assert_eq!(bucket.object(&format!("{REMOVAL_REQUESTS}{}", id(1))).unwrap(), request_before);
        assert_eq!(packed(&bucket, &files).len(), 0);
    }

    #[test]
    fn a_requested_submission_in_a_pack_is_taken_out_and_every_other_one_stays() {
        let (bucket, files, pack) = bucket_with_a_pack(4);
        let pack_before = bucket.object(&pack).unwrap();
        request(&bucket, 1);
        // Folder markers, which some tools create, are neither packs nor requests.
        for marker in ["packs/", "packs/live/", REMOVAL_REQUESTS, "removals/requested/2026/"] {
            bucket.store(marker, b"", now());
        }

        let first = run_once(&bucket);

        // Every submission has the same export; they differ only in their ids. Exactly the
        // requested one is gone, and the others read back as their files.
        let found = packed(&bucket, &files);
        assert_eq!(found.keys().cloned().collect::<Vec<_>>(), [name(0), name(2), name(3)]);
        // The pack keeps its key.
        assert_eq!(pack_keys(&bucket), [pack.as_str()]);
        assert!(bucket.object(&pack).unwrap().len() < pack_before.len());
        assert_eq!(first.removals.removed[0].packs_rewritten, [pack.as_str()]);
        assert_eq!((first.removals.unanswered, first.removals.stopped), (NOTHING.to_vec(), None));
        assert!(!has_receipt(&bucket, 1));

        let second = run_once(&bucket);
        assert!(has_receipt(&bucket, 1));
        assert_eq!((second.removals.unanswered, second.removals.stopped), (NOTHING.to_vec(), None));
        assert_eq!(packed(&bucket, &files).len(), 3);
    }

    #[test]
    fn every_copy_of_a_submission_is_removed_wherever_it_is() {
        // In two packs and in loose/: what a copy the intake made twice can lead to.
        let (bucket, files, pack) = bucket_with_a_pack(3);
        let second = "packs/backfill/2026/11/08/18-0.pack";
        bucket.store(second, &bucket.object(&pack).unwrap(), now());
        bucket.store(&format!("{LOOSE}{}", name(1)), &files[&name(1)], hours_ago(1));
        request(&bucket, 1);

        let report = run_once(&bucket);

        let found = packed(&bucket, &files);
        assert_eq!(found.keys().cloned().collect::<Vec<_>>(), [name(0), name(2)]);
        assert!(found.values().all(|packs| *packs == 2));
        assert_eq!(bucket.keys(LOOSE), [] as [&str; 0]);
        assert_eq!(report.removals.removed[0].loose_deleted, 1);
        assert_eq!(report.removals.removed[0].packs_rewritten, [second, pack.as_str()]);
        run_once(&bucket);
        assert!(has_receipt(&bucket, 1));
    }

    #[test]
    fn two_requests_for_one_pack_are_both_worked_on_in_one_run() {
        let (bucket, files, _) = bucket_with_a_pack(3);
        request(&bucket, 0);
        request(&bucket, 2);

        let report = run_once(&bucket);

        assert_eq!(packed(&bucket, &files).keys().cloned().collect::<Vec<_>>(), [name(1)]);
        assert_eq!(report.removals.removed.len(), 2);
        assert_eq!(run_once(&bucket).removals.answered, [id(0), id(2)]);
        assert_eq!(packed(&bucket, &files).keys().cloned().collect::<Vec<_>>(), [name(1)]);
    }

    #[test]
    fn removing_the_last_submission_leaves_an_empty_pack_that_still_opens() {
        let (bucket, files, pack) = bucket_with_a_pack(1);
        request(&bucket, 0);

        run_once(&bucket);
        run_once(&bucket);

        assert_eq!(pack_keys(&bucket), [pack.as_str()]);
        assert_eq!(packed(&bucket, &files).len(), 0);
        assert!(has_receipt(&bucket, 0));
    }

    /// A submission is the one its name says, in a pack as in loose/: not the one its file
    /// claims to be, and not one whose id merely ends the same.
    #[test]
    fn a_submission_is_matched_by_the_id_in_its_name_and_by_nothing_else() {
        let bucket = MemoryBucket::new(now());
        let mut files = BTreeMap::new();
        for (minute, name_id, file_id) in [(1, "aabb", "aabb"), (2, "bb", "bb"), (3, "cc", "dd")] {
            let (name, _) = submission(minute, name_id);
            let (_, file) = submission(minute, file_id);
            bucket.store(&format!("{LOOSE}{name}"), &file, hours_ago(49));
            files.insert(name, file);
        }
        run_once(&bucket);
        assert_eq!(packed(&bucket, &files).len(), 3);
        // "bb" also ends the name of "aabb". "dd" is what the third file says it is, under
        // the name of "cc".
        request_for(&bucket, "bb", "x");
        request_for(&bucket, "dd", "x");

        let report = run_once(&bucket);

        let left: Vec<String> = packed(&bucket, &files).into_keys().collect();
        assert_eq!(left, [submission(1, "aabb").0, submission(3, "cc").0]);
        assert_eq!(report.removals.removed.iter().map(|removed| removed.id.as_str()).collect::<Vec<_>>(), ["bb"]);
        assert!(report.removals.unanswered.len() == 1 && report.removals.unanswered[0].starts_with("dd: found in neither"));
    }

    #[test]
    fn a_submission_found_nowhere_is_not_answered_unless_its_removal_was_started() {
        let (bucket, mut files, _) = bucket_with_a_pack(2);
        request(&bucket, 7);

        // Nothing says this submission was ever removed: the request stays open, and the rest
        // of the run goes on.
        let (late_name, late_file) = submission(40, "late");
        bucket.store(&format!("{LOOSE}{late_name}"), &late_file, hours_ago(49));
        files.insert(late_name, late_file);
        let report = run_once(&bucket);
        assert!(!has_receipt(&bucket, 7));
        assert_eq!(bucket.keys(REMOVAL_INTENTS), [] as [&str; 0]);
        assert_eq!(report.removals.unanswered.len(), 1);
        assert!(report.removals.unanswered[0].contains(&id(7)) && report.removals.unanswered[0].contains("found in neither"));
        assert_eq!(report.packed.packs.len(), 1);
        assert_eq!(packed(&bucket, &files).len(), 3);

        // With the record of a removal that an earlier run started.
        bucket.store(&format!("{REMOVAL_INTENTS}{}", id(7)), b"{}", now());
        let report = run_once(&bucket);
        assert!(has_receipt(&bucket, 7));
        assert_eq!(report.removals.unanswered, NOTHING);
    }

    #[test]
    fn an_answered_request_is_not_looked_at_again() {
        let (bucket, _) = bucket_with(2, 49);
        request(&bucket, 0);
        bucket.store(&format!("{REMOVAL_RECEIPTS}{}", id(0)), b"{}", now());

        let report = run_once(&bucket);

        // The loose object is not what the receipt says, which is not the compactor's to
        // repair: it is neither deleted nor packed.
        assert_eq!(report.removals, Removals::default());
        assert_eq!(bucket.keys(LOOSE), [format!("{LOOSE}{}", name(0))]);
        assert_eq!(bucket.keys(REMOVAL_INTENTS), [] as [&str; 0]);
    }

    #[test]
    fn a_request_that_cannot_be_read_or_names_another_submission_stays_open() {
        for body in [
            &b"not json"[..],
            br#"{"schema_version":2,"id":"00000000000000000000000000000001"}"#,
            br#"{"schema_version":1,"id":"00000000000000000000000000000000"}"#,
            br#"{"schema_version":1}"#,
        ] {
            let (bucket, files, _) = bucket_with_a_pack(2);
            bucket.store(&format!("{LOOSE}{}", name(1)), &files[&name(1)], hours_ago(49));
            bucket.store(&format!("{REMOVAL_REQUESTS}{}", id(1)), body, now());

            let report = run_once(&bucket);

            assert_eq!(report.removals.unanswered.len(), 1);
            assert_eq!(bucket.keys(REMOVAL_INTENTS), [] as [&str; 0]);
            // Nothing of it was touched, and it was not packed a second time either.
            assert_eq!(packed(&bucket, &files), BTreeMap::from([(name(0), 1), (name(1), 1)]));
            assert_eq!(bucket.keys(LOOSE), [format!("{LOOSE}{}", name(1))]);
        }
    }

    #[test]
    fn a_pack_whose_index_cannot_be_read_stops_the_removals_and_not_the_packing() {
        let (bucket, mut files, _) = bucket_with_a_pack(2);
        bucket.store(&format!("{LOOSE}{}", name(1)), &files[&name(1)], hours_ago(1));
        let unreadable = "packs/backfill/2026/11/08/18-0.pack";
        bucket.store(unreadable, b"this is not a pack, so what it holds is unknown", now());
        request(&bucket, 1);
        let (settled_name, settled_file) = submission(40, "settled");
        bucket.store(&format!("{LOOSE}{settled_name}"), &settled_file, hours_ago(49));
        files.insert(settled_name.clone(), settled_file);

        let report = run_once(&bucket);

        // Nothing is concluded about the requested submission, and nothing of it is touched.
        assert!(
            report.removals.stopped.as_ref().is_some_and(|reason| reason.contains(unreadable)),
            "{:?}",
            report.removals
        );
        assert_eq!(bucket.keys(LOOSE), [format!("{LOOSE}{}", name(1))]);
        assert_eq!(bucket.keys(REMOVAL_INTENTS), [] as [&str; 0]);
        // What had settled is packed all the same.
        assert_eq!(report.packed.packs.len(), 1);
        bucket.remove(unreadable);
        assert_eq!(packed(&bucket, &files), BTreeMap::from([(name(0), 1), (name(1), 1), (settled_name, 1)]));
    }

    #[test]
    fn a_pack_replaced_after_it_was_listed_is_not_rewritten() {
        let (bucket, _, pack) = bucket_with_a_pack(3);
        request(&bucket, 1);
        // Another pack takes its place between the reading of its index and the rewrite.
        let other = {
            let (other_bucket, _, other_pack) = bucket_with_a_pack(2);
            other_bucket.object(&other_pack).unwrap()
        };
        bucket.before_get(&pack, &other);

        let report = run_once(&bucket);

        assert!(
            report
                .removals
                .stopped
                .as_ref()
                .is_some_and(|reason| reason.contains("not the pack that was listed")),
            "{:?}",
            report.removals
        );
        assert_eq!(bucket.object(&pack).unwrap(), other);
    }

    #[test]
    fn a_loose_object_that_changed_after_it_was_listed_is_not_deleted_for_a_removal() {
        let (bucket, files) = bucket_with(2, 47);
        request(&bucket, 1);
        let changed = format!("{LOOSE}{}", name(1));
        bucket.after_listing(LOOSE, &changed, b"other bytes under the same key");

        let report = run_once(&bucket);

        assert!(
            report
                .removals
                .stopped
                .as_ref()
                .is_some_and(|reason| reason.contains("changed since it was listed")),
            "{:?}",
            report.removals
        );
        assert_eq!(bucket.object(&changed).unwrap(), b"other bytes under the same key");
        let _ = files;
    }

    /// The upload carries a checksum the provider enforces. Should a rewritten pack be stored
    /// wrongly all the same, or not at all, the removal must not count as made.
    #[test]
    fn a_rewritten_pack_the_bucket_did_not_store_as_sent_stops_the_removals() {
        for fault in [PutFault::Damaged, PutFault::NotStored] {
            let (bucket, _, pack) = bucket_with_a_pack(3);
            request(&bucket, 1);
            bucket.fail_put(&pack, fault);

            let report = run_once(&bucket);

            assert!(
                report
                    .removals
                    .stopped
                    .as_ref()
                    .is_some_and(|reason| reason.contains("does not hold the rewritten pack")),
                "{fault:?}: {:?}",
                report.removals
            );
            assert_eq!(report.removals.removed, [], "{fault:?}");
        }
    }

    /// A pack whose loose objects are not deleted yet must be finished before a removal
    /// rewrites it: the intent names the pack by the checksum it had.
    #[test]
    fn an_interrupted_pack_is_finished_before_a_removal_rewrites_it() {
        let (bucket, files) = bucket_with(3, 49);
        stopped_after_the_upload(&bucket);
        request(&bucket, 1);

        run_once(&bucket);

        assert_eq!(bucket.keys("intents/"), [] as [&str; 0]);
        assert_eq!(bucket.keys(LOOSE), [] as [&str; 0]);
        assert_eq!(packed(&bucket, &files).keys().cloned().collect::<Vec<_>>(), [name(0), name(2)]);
    }

    /// The two things a removal must never do, wherever a run stops, and wherever the run
    /// after it stops too: lose another submission, and answer while the requested one is
    /// still stored somewhere.
    #[test]
    fn a_removal_stopped_at_any_request_keeps_the_others_and_answers_only_when_all_is_gone() {
        // Submission 0 is in a pack and, copied a second time, in loose/. Submission 1 is in
        // the pack only. Both are to go; 2 and 3 stay.
        let setup = || {
            let (bucket, files, _) = bucket_with_a_pack(4);
            bucket.store(&format!("{LOOSE}{}", name(0)), &files[&name(0)], hours_ago(1));
            request(&bucket, 0);
            request(&bucket, 1);
            (bucket, files)
        };
        let check = |bucket: &MemoryBucket, files: &BTreeMap<String, Vec<u8>>, when: &str| {
            let found = packed(bucket, files);
            for kept in [2, 3] {
                assert_eq!(found.get(&name(kept)), Some(&1), "{when}: submission {kept}");
            }
            for removed in [0, 1] {
                let stored = found.contains_key(&name(removed)) || bucket.object(&format!("{LOOSE}{}", name(removed))).is_some();
                assert!(
                    !(has_receipt(bucket, removed) && stored),
                    "{when}: submission {removed} was answered while it is still stored"
                );
            }
        };
        let finished = |bucket: &MemoryBucket, files: &BTreeMap<String, Vec<u8>>, when: &str| {
            // One run removes what is left, the next answers.
            for _ in 0..2 {
                let report = run(bucket, &config(u64::MAX), now()).unwrap_or_else(|error| panic!("{when}: a later run failed: {error:#}"));
                assert_eq!((report.removals.unanswered, report.removals.stopped), (NOTHING.to_vec(), None), "{when}");
                check(bucket, files, when);
            }
            assert!(has_receipt(bucket, 0) && has_receipt(bucket, 1), "{when}");
            assert_eq!(bucket.keys(LOOSE), [] as [&str; 0], "{when}");
        };

        let stopped_runs = stop_everywhere(setup, &config(u64::MAX), check, finished);
        assert!(stopped_runs > 15, "only {stopped_runs} requests were failed");
    }
}
