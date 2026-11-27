//! What the tests of packing and of removal share: a bucket of loose submissions, and a check
//! of everything the bucket's packs hold.

use crate::bucket::LOOSE;
use crate::memory_bucket::MemoryBucket;
use crate::Config;
use jiff::{SignedDuration, Timestamp};
use std::collections::BTreeMap;
use trace_pack::pack::{PackOptions, PackReader};
use trace_pack::record::Record;

pub fn now() -> Timestamp {
    "2026-11-24T22:30:00Z".parse().unwrap()
}

pub fn hours_ago(hours: i64) -> Timestamp {
    now().checked_sub(SignedDuration::from_hours(hours)).unwrap()
}

pub fn config(pack_input_bytes: u64) -> Config {
    Config {
        writer: "live".into(),
        settle: SignedDuration::from_hours(48),
        pack_input_bytes,
        build_input_bytes: u64::MAX,
        options: PackOptions::default(),
    }
}

/// The id of the submission `bucket_with` stores at this position.
pub fn id(index: u32) -> String {
    format!("{index:032x}")
}

/// A submission as the intake stores it. Its export string holds no export, so a pack keeps
/// the file whole and a test can compare what it reads back byte for byte.
pub fn submission(minute: u32, id: &str) -> (String, Vec<u8>) {
    let name = format!("2026-11/2026-11-08_18{minute:02}49Z_{id}.json");
    let file = format!(
        r#"{{"schema_version":2,"id":"{id}","contributor_id":"contributor-{id}","received_at":"2026-11-08T18:{minute:02}:49Z","bytes":9,"export_string":"not one"}}"#
    );
    (name, file.into_bytes())
}

/// A bucket holding `count` loose submissions stored `stored_hours_ago`, and their files by
/// name.
pub fn bucket_with(count: u32, stored_hours_ago: i64) -> (MemoryBucket, BTreeMap<String, Vec<u8>>) {
    let bucket = MemoryBucket::new(now());
    let mut files = BTreeMap::new();
    for index in 0..count {
        let (name, file) = submission(index, &id(index));
        // The later a submission was received, the earlier the bucket stored it: the order in
        // a pack must come from the names, not from the bucket's clock.
        let stored_at = hours_ago(stored_hours_ago).checked_sub(SignedDuration::from_secs(index.into())).unwrap();
        bucket.store(&format!("{LOOSE}{name}"), &file, stored_at);
        files.insert(name, file);
    }
    (bucket, files)
}

/// The keys of the packs in the bucket, in the order of their keys. Folder markers, which
/// some tools create, are not packs.
pub fn pack_keys(bucket: &MemoryBucket) -> Vec<String> {
    bucket.keys("packs/").into_iter().filter(|key| !key.ends_with('/')).collect()
}

/// In how many stored packs each submission is. Every pack in the bucket must open, and every
/// record in it must be the file it was made from.
pub fn packed(bucket: &MemoryBucket, files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, usize> {
    let mut found = BTreeMap::new();
    for key in pack_keys(bucket) {
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

/// Leaves the bucket as a run does that stored and confirmed its pack and stopped before it
/// deleted anything: the intent and every loose object are there for the next run. The bucket
/// must hold what `bucket_with(3, 49)` gives.
pub fn stopped_after_the_upload(bucket: &MemoryBucket) {
    use crate::memory_bucket::Fault;
    let first_delete = (0..)
        .find(|call| {
            let (probe, _) = bucket_with(3, 49);
            probe.fail_call(*call, Fault::Refused);
            let _ = crate::run(&probe, &config(u64::MAX), now());
            probe.restore() && !pack_keys(&probe).is_empty()
        })
        .unwrap();
    bucket.fail_call(first_delete, Fault::Refused);
    crate::run(bucket, &config(u64::MAX), now()).unwrap_err();
    assert!(bucket.restore());
    assert_eq!((bucket.keys(LOOSE).len(), pack_keys(bucket).len()), (3, 1));
}

/// Runs `setup` again and again, each time stopping the run at another request, in both ways
/// a request can fail, and then stopping the run after it at each of its requests too.
/// `check` is called after every stopped run, and `finished` once nothing is stopped any
/// more: it is to run once more itself and look at the result. Returns how many first runs
/// were stopped.
pub fn stop_everywhere(
    setup: impl Fn() -> (MemoryBucket, BTreeMap<String, Vec<u8>>),
    config: &Config,
    check: impl Fn(&MemoryBucket, &BTreeMap<String, Vec<u8>>, &str),
    finished: impl Fn(&MemoryBucket, &BTreeMap<String, Vec<u8>>, &str),
) -> usize {
    use crate::memory_bucket::Fault;
    // One run that is stopped at `call`. Reports whether the fault was reached.
    let stop_at = |bucket: &MemoryBucket, call: usize, fault: Fault, when: &str| {
        bucket.fail_call(call, fault);
        let outcome = crate::run(bucket, config, now());
        let stopped = bucket.restore();
        assert_eq!(outcome.is_err(), stopped, "{when}: {outcome:?}");
        stopped
    };

    let mut stopped_runs = 0;
    for first_fault in [Fault::Refused, Fault::AnswerLost] {
        for first in 0.. {
            let when = format!("first run stopped at request {first} ({first_fault:?})");
            let (bucket, files) = setup();
            let stopped = stop_at(&bucket, first, first_fault, &when);
            check(&bucket, &files, &when);
            if !stopped {
                break;
            }
            stopped_runs += 1;
            finished(&bucket, &files, &when);

            // The same again, with the run that follows stopped as well.
            for second_fault in [Fault::Refused, Fault::AnswerLost] {
                for second in 0.. {
                    let when = format!("{when}, the next at request {second} ({second_fault:?})");
                    let (bucket, files) = setup();
                    stop_at(&bucket, first, first_fault, &when);
                    let stopped = stop_at(&bucket, second, second_fault, &when);
                    check(&bucket, &files, &when);
                    if !stopped {
                        break;
                    }
                    finished(&bucket, &files, &when);
                }
            }
        }
    }
    stopped_runs
}
