//! Removals: a submission the intake wants gone leaves `loose/` and every pack that holds it.
//!
//! The intake writes `removals/requested/<id>` when a rejection of an accepted submission has
//! become final, and purges its own receipt of the submission once `removals/done/<id>` is
//! listed. For each request without that answer, in this order:
//!
//! 1. Find every occurrence of the submission: in `loose/`, and in every pack, by reading
//!    every pack's index.
//! 2. Record that the removal starts: `removals/started/<id>`, naming what was found.
//! 3. Delete the loose object. Rewrite each pack that holds the submission without it.
//! 4. Write `removals/done/<id>`.
//!
//! The answer is written last, and only for a removal that was recorded as started. Finding
//! the submission nowhere is not an answer by itself: that is also what a mistake in step 1
//! would look like.

use crate::bucket::{
    delete_unchanged, md5_hex, submission_id, submission_name, Bucket, Listed, LOOSE, PACKS, REMOVAL_INTENTS, REMOVAL_RECEIPTS, REMOVAL_REQUESTS,
};
use anyhow::{anyhow, bail, Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use trace_pack::pack::{self, Entry};

/// What the removal step of one run did.
#[derive(Debug, Default, PartialEq)]
pub struct Removals {
    pub answered: Vec<Removed>,
    /// One line for each request that stays open, with the reason. Each needs someone to
    /// look: the next run will find the same.
    pub unanswered: Vec<String>,
}

/// One answered request.
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

fn ids_under(listed: Vec<Listed>, prefix: &str) -> BTreeSet<String> {
    listed.into_iter().map(|object| object.key[prefix.len()..].to_owned()).collect()
}

/// Answers every request that has no receipt yet. `level` compresses the index of a rewritten
/// pack.
///
/// No pack may be mid-upload: the caller finishes an interrupted pack first.
pub fn answer(bucket: &dyn Bucket, requested: &BTreeSet<String>, level: i32) -> Result<Removals> {
    let answered = ids_under(bucket.list(REMOVAL_RECEIPTS)?, REMOVAL_RECEIPTS);
    let open: Vec<&String> = requested.difference(&answered).collect();
    let mut removals = Removals::default();
    if open.is_empty() {
        return Ok(removals);
    }

    // Where every stored submission is. Read once for all requests; a pack that is rewritten
    // below is brought up to date here.
    let loose = bucket.list(LOOSE)?;
    let mut packs = read_pack_indexes(bucket)?;

    for id in open {
        match remove(bucket, id, &loose, &mut packs, level)? {
            Ok(removed) => removals.answered.push(removed),
            Err(reason) => removals.unanswered.push(format!("{id}: {reason}")),
        }
    }
    Ok(removals)
}

/// A pack in the bucket and what its index lists.
struct StoredPack {
    key: String,
    etag: String,
    entries: Vec<Entry>,
}

impl StoredPack {
    fn holds(&self, id: &str) -> bool {
        self.entries.iter().any(|entry| submission_id(&entry.name) == id)
    }
}

/// The index of every pack of every writer. One that cannot be read is an error: without it
/// nothing can be said about where a submission is not.
fn read_pack_indexes(bucket: &dyn Bucket) -> Result<Vec<StoredPack>> {
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
            Ok(StoredPack {
                key: listed.key,
                etag: listed.etag,
                entries,
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

/// The record that a removal was started: step 2. Written once and kept for good.
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

/// Removes one submission and answers its request. The inner `Err` is the reason the request
/// stays open; the outer one stops the run.
fn remove(bucket: &dyn Bucket, id: &str, loose: &[Listed], packs: &mut [StoredPack], level: i32) -> Result<Result<Removed, String>> {
    // The request must be one this version understands, and about the id its key names.
    let Some(request) = bucket.get(&format!("{REMOVAL_REQUESTS}{id}"))? else {
        return Ok(Err("its request is gone since it was listed".into()));
    };
    match serde_json::from_slice::<Request>(&request) {
        Ok(request) if request.schema_version == 1 && request.id == id => {}
        _ => return Ok(Err("its request is not one this version reads, or names another submission".into())),
    }

    // Every occurrence, by id.
    let loose_copies: Vec<&Listed> = loose
        .iter()
        .filter(|object| submission_name(&object.key).is_some_and(|name| submission_id(name) == id))
        .collect();
    let holding: Vec<usize> = (0..packs.len()).filter(|&index| packs[index].holds(id)).collect();

    let intent_key = format!("{REMOVAL_INTENTS}{id}");
    let started = bucket.etag(&intent_key)?.is_some();
    if loose_copies.is_empty() && holding.is_empty() {
        // An earlier run recorded the removal, deleted everything and stopped before the
        // receipt. Without that record, nothing found means nothing is known.
        if !started {
            return Ok(Err("found in neither loose/ nor any pack, and no removal of it was started".into()));
        }
    } else if !started {
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

    // Last: from here on the intake forgets the submission.
    let receipt = Receipt { schema_version: 1, id };
    bucket.put(&format!("{REMOVAL_RECEIPTS}{id}"), &serde_json::to_vec_pretty(&receipt)?)?;
    Ok(Ok(Removed {
        id: id.to_owned(),
        loose_deleted: loose_copies.len(),
        packs_rewritten: holding.iter().map(|&index| packs[index].key.clone()).collect(),
    }))
}

/// Replaces a pack with itself minus one submission, under the same key.
///
/// The upload replaces the only copy of every other submission in the pack. The old pack
/// must therefore be the object whose index was read, and [`pack::without`] compares every
/// frame it keeps with the old pack before anything is uploaded.
fn rewrite_without(bucket: &dyn Bucket, stored: &mut StoredPack, id: &str, level: i32) -> Result<()> {
    let old = bucket
        .get(&stored.key)?
        .with_context(|| format!("{}: gone since its index was read", stored.key))?;
    if md5_hex(&old) != stored.etag {
        bail!("{}: what was downloaded is not the pack whose index was read", stored.key);
    }
    let rewritten = pack::without(&old, level, |entry| submission_id(&entry.name) == id).map_err(|message| anyhow!("{}: {message}", stored.key))?;
    drop(old);

    bucket.put(&stored.key, &rewritten)?;
    let etag = md5_hex(&rewritten);
    if bucket.etag(&stored.key)?.as_deref() != Some(etag.as_str()) {
        bail!("{}: the bucket does not hold the rewritten pack", stored.key);
    }
    stored.entries.retain(|entry| submission_id(&entry.name) != id);
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

    /// The request the intake writes for this submission.
    fn request(bucket: &MemoryBucket, index: u32) {
        let body = format!(
            "{{\n  \"schema_version\": 1,\n  \"id\": \"{}\",\n  \"name\": \"{}\"\n}}\n",
            id(index),
            name(index)
        );
        bucket.store(&format!("{REMOVAL_REQUESTS}{}", id(index)), body.as_bytes(), now());
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

    #[test]
    fn a_requested_submission_still_in_loose_is_deleted_and_answered() {
        let (bucket, files) = bucket_with(3, 47);
        request(&bucket, 1);
        let request_before = bucket.object(&format!("{REMOVAL_REQUESTS}{}", id(1))).unwrap();

        let report = run_once(&bucket);

        assert_eq!(bucket.keys(LOOSE), [format!("{LOOSE}{}", name(0)), format!("{LOOSE}{}", name(2))]);
        assert!(has_receipt(&bucket, 1));
        assert_eq!(bucket.keys(REMOVAL_INTENTS), [format!("{REMOVAL_INTENTS}{}", id(1))]);
        // The request is the intake's: it is neither changed nor removed.
        assert_eq!(bucket.object(&format!("{REMOVAL_REQUESTS}{}", id(1))).unwrap(), request_before);
        assert_eq!(report.removals.answered.len(), 1);
        assert_eq!(
            (report.removals.answered[0].loose_deleted, report.removals.answered[0].packs_rewritten.len()),
            (1, 0)
        );
        assert_eq!(packed(&bucket, &files).len(), 0);
    }

    #[test]
    fn a_requested_submission_in_a_pack_is_taken_out_and_every_other_one_stays() {
        let (bucket, files, pack) = bucket_with_a_pack(4);
        let pack_before = bucket.object(&pack).unwrap();
        request(&bucket, 1);

        let report = run_once(&bucket);

        // Every submission has the same export; they differ only in their ids. Exactly the
        // requested one is gone, and the others read back as their files.
        let found = packed(&bucket, &files);
        assert_eq!(found.keys().cloned().collect::<Vec<_>>(), [name(0), name(2), name(3)]);
        // The pack keeps its key.
        assert_eq!(pack_keys(&bucket), [pack.as_str()]);
        assert!(bucket.object(&pack).unwrap().len() < pack_before.len());
        assert!(has_receipt(&bucket, 1));
        assert_eq!(report.removals.answered[0].packs_rewritten, [pack.as_str()]);
        assert_eq!(report.removals.unanswered, [] as [String; 0]);
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
        assert!(has_receipt(&bucket, 1));
        assert_eq!(report.removals.answered[0].loose_deleted, 1);
        assert_eq!(report.removals.answered[0].packs_rewritten, [second, pack.as_str()]);
    }

    #[test]
    fn two_requests_for_one_pack_are_both_answered_in_one_run() {
        let (bucket, files, _) = bucket_with_a_pack(3);
        request(&bucket, 0);
        request(&bucket, 2);

        let report = run_once(&bucket);

        assert_eq!(packed(&bucket, &files).keys().cloned().collect::<Vec<_>>(), [name(1)]);
        assert!(has_receipt(&bucket, 0) && has_receipt(&bucket, 2));
        assert_eq!(report.removals.answered.len(), 2);
    }

    #[test]
    fn removing_the_last_submission_leaves_an_empty_pack_that_still_opens() {
        let (bucket, files, pack) = bucket_with_a_pack(1);
        request(&bucket, 0);

        run_once(&bucket);

        assert_eq!(pack_keys(&bucket), [pack.as_str()]);
        assert_eq!(packed(&bucket, &files).len(), 0);
        assert!(has_receipt(&bucket, 0));
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

        // With the record of a removal that an earlier run started and did not answer.
        bucket.store(&format!("{REMOVAL_INTENTS}{}", id(7)), b"{}", now());
        let report = run_once(&bucket);
        assert!(has_receipt(&bucket, 7));
        assert_eq!(report.removals.unanswered, [] as [String; 0]);
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
            assert!(!has_receipt(&bucket, 1));
            // Nothing of it was touched, and it was not packed a second time either.
            assert_eq!(packed(&bucket, &files), BTreeMap::from([(name(0), 1), (name(1), 1)]));
            assert_eq!(bucket.keys(LOOSE), [format!("{LOOSE}{}", name(1))]);
        }
    }

    #[test]
    fn a_pack_whose_index_cannot_be_read_stops_every_removal() {
        let (bucket, files, _) = bucket_with_a_pack(2);
        bucket.store(&format!("{LOOSE}{}", name(1)), &files[&name(1)], hours_ago(1));
        let unreadable = "packs/backfill/2026/11/08/18-0.pack";
        bucket.store(unreadable, b"this is not a pack, so what it holds is unknown", now());
        request(&bucket, 1);

        let error = run(&bucket, &config(u64::MAX), now()).unwrap_err();

        assert!(format!("{error:#}").contains(unreadable), "{error:#}");
        assert!(!has_receipt(&bucket, 1));
        assert_eq!(bucket.keys(LOOSE), [format!("{LOOSE}{}", name(1))]);
        assert_eq!(bucket.keys(REMOVAL_INTENTS), [] as [&str; 0]);
        bucket.remove(unreadable);
        assert_eq!(packed(&bucket, &files), BTreeMap::from([(name(0), 1), (name(1), 1)]));
    }

    #[test]
    fn a_pack_replaced_after_its_index_was_read_is_not_rewritten() {
        let (bucket, _, pack) = bucket_with_a_pack(3);
        request(&bucket, 1);
        // Another pack takes its place between the reading of its index and the rewrite.
        let other = {
            let (other_bucket, _, other_pack) = bucket_with_a_pack(2);
            other_bucket.object(&other_pack).unwrap()
        };
        bucket.before_get(&pack, &other);

        let error = run(&bucket, &config(u64::MAX), now()).unwrap_err();

        assert!(format!("{error:#}").contains("not the pack whose index was read"), "{error:#}");
        assert_eq!(bucket.object(&pack).unwrap(), other);
        assert!(!has_receipt(&bucket, 1));
    }

    /// The upload carries a checksum the provider enforces. Should a rewritten pack be stored
    /// wrongly all the same, or not at all, the submission must not be reported as removed.
    #[test]
    fn a_rewritten_pack_the_bucket_did_not_store_as_sent_is_not_answered() {
        for fault in [PutFault::Damaged, PutFault::NotStored] {
            let (bucket, _, pack) = bucket_with_a_pack(3);
            request(&bucket, 1);
            bucket.fail_put(&pack, fault);

            let error = run(&bucket, &config(u64::MAX), now()).unwrap_err();

            assert!(format!("{error:#}").contains("does not hold the rewritten pack"), "{fault:?}: {error:#}");
            assert!(!has_receipt(&bucket, 1), "{fault:?}");
        }
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
            let report = run(bucket, &config(u64::MAX), now()).unwrap_or_else(|error| panic!("{when}: the last run failed: {error:#}"));
            check(bucket, files, when);
            assert!(has_receipt(bucket, 0) && has_receipt(bucket, 1), "{when}");
            assert_eq!(report.removals.unanswered, [] as [String; 0], "{when}");
            assert_eq!(bucket.keys(LOOSE), [] as [&str; 0], "{when}");
        };

        let stopped_runs = stop_everywhere(setup, &config(u64::MAX), check, finished);
        assert!(stopped_runs > 15, "only {stopped_runs} requests were failed");
    }
}
