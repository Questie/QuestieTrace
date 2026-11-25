//! The backfill: packs the submissions of the old archive into a local directory.
//!
//! The old archive is one JSON object per submission, `submissions/<name>`, in a bucket and,
//! in part, in a local trace-data checkout. A run packs every submission that is in no pack
//! of the output directory yet, one pack per hour the submissions were received in:
//!
//! ```text
//! <out>/packs/backfill/YYYY/MM/DD/HH-<md5>.pack
//! ```
//!
//! It deletes nothing and writes nothing to the source. It can be stopped at any moment and
//! run again: a pack is a whole file or is not there, and what a run finds packed it skips.
//! A submission that reaches the source after its hour was packed gets a pack of its own for
//! that hour in a later run.

use crate::bucket::md5_hex;
use crate::packing::{BuildBudget, PackReport};
use anyhow::{anyhow, bail, Context, Result};
use jiff::{SignedDuration, Timestamp};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use trace_data::source::list_submissions;
use trace_pack::pack::{self, build_frame, PackOptions, PackWriter, Scratch};
use trace_pack::record::Encoding;

/// Where the old archive's bucket keeps its submissions.
const SUBMISSIONS: &str = "submissions/";

/// Where a run writes, under the output directory. `backfill` is this writer's name: the
/// compactor on the server writes under `packs/live/`.
const PACKS: &str = "packs/backfill";

pub struct Config {
    /// The directory the packs go to. Uploading `<out>/packs` is another tool's job.
    pub out: PathBuf,
    /// Further directories whose packs count as packed, for an archive that is on more than
    /// one disk. They are only read.
    pub also_packed: Vec<PathBuf>,
    /// An hour is packed once it ended this long ago. Submissions of a recent hour can still
    /// be accepted, or rejected and deleted from the source.
    pub settle: SignedDuration,
    /// The most JSON one pack is built from, in bytes. An hour with more gets several packs.
    pub pack_input_bytes: u64,
    /// The most JSON that is being built into frames at one moment, in bytes. See
    /// [`crate::Config::build_input_bytes`].
    pub build_input_bytes: u64,
    /// Only submissions whose name starts with this, such as `2026-11/2026-11-08`.
    pub only: Option<String>,
    pub options: PackOptions,
}

/// The part of a bucket the backfill may use. The old archive is in production: nothing here
/// can write to it or delete from it.
pub trait ReadBucket: Sync {
    /// The name, size and ETag of every object under `prefix`.
    fn list(&self, prefix: &str) -> Result<Vec<(String, u64, String)>>;
    /// The whole object. None when the key is absent.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
}

/// Where submissions are read from: the old bucket, a local trace-data checkout, or both.
///
/// With a bucket, the bucket says which submissions exist, and a local file is used in place
/// of a download when it is the object the bucket lists. Without one, the checkout says.
pub struct Source<'a> {
    pub bucket: Option<&'a dyn ReadBucket>,
    /// A trace-data checkout: `<dir>/submissions/<yyyy-mm>/*.json`.
    pub local: Option<PathBuf>,
}

/// One submission of the source.
struct Stored {
    /// `<yyyy-mm>/<time received>_<id>.json`.
    name: String,
    size: u64,
    /// The bucket's checksum, when the bucket listed it.
    etag: Option<String>,
}

impl Source<'_> {
    /// The submissions whose names start with `only`. A bucket is asked for no others: it
    /// holds several hundred thousand.
    fn list(&self, only: &str) -> Result<Vec<Stored>> {
        if let Some(bucket) = self.bucket {
            let listed = bucket.list(&format!("{SUBMISSIONS}{only}"))?;
            return Ok(listed
                .into_iter()
                .filter_map(|(key, size, etag)| {
                    let name = key.strip_prefix(SUBMISSIONS).filter(|name| !name.is_empty() && !name.ends_with('/'))?;
                    Some(Stored {
                        name: name.to_owned(),
                        size,
                        etag: Some(etag),
                    })
                })
                .collect());
        }
        let local = self
            .local
            .as_deref()
            .context("the backfill needs a bucket or a trace-data checkout to read from")?;
        let files = list_submissions(local).with_context(|| format!("listing {}/submissions", local.display()))?;
        files
            .into_iter()
            .filter(|file| file.key.starts_with(only))
            .map(|file| {
                let size = std::fs::metadata(&file.path).with_context(|| file.key.clone())?.len();
                Ok(Stored {
                    name: file.key,
                    size,
                    etag: None,
                })
            })
            .collect()
    }

    /// The submission's file. The inner `Err` is a fault of this submission alone; the outer
    /// one is the bucket or the disk failing.
    fn read(&self, stored: &Stored) -> Result<Result<Vec<u8>, String>> {
        // A local file, when it is the object the bucket listed.
        if let Some(local) = &self.local {
            match std::fs::read(local.join(SUBMISSIONS).join(&stored.name)) {
                Ok(file) if stored.etag.as_ref().is_none_or(|etag| md5_hex(&file) == *etag) => return Ok(Ok(file)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).with_context(|| stored.name.clone()),
            }
        }
        let (Some(bucket), Some(etag)) = (self.bucket, &stored.etag) else {
            return Ok(Err("its file is gone since it was listed".into()));
        };
        Ok(match bucket.get(&format!("{SUBMISSIONS}{}", stored.name))? {
            None => Err("gone since it was listed".into()),
            Some(file) if md5_hex(&file) == *etag => Ok(file),
            Some(_) => Err(format!(
                "its ETag {etag} is not the MD5 of what was downloaded: it changed, or was not stored in one part"
            )),
        })
    }
}

/// What one run did.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub packs: Vec<PackReport>,
    /// Submissions found in a pack of the output directory, and so not packed again.
    pub already_packed: usize,
    /// Submissions of hours that have not settled, left for a later run.
    pub not_settled: usize,
    /// One line for each submission that could not be packed, with the reason.
    pub left: Vec<String>,
    /// Submissions that are in a pack and no longer in the source: rejected and deleted
    /// there since. Only reported; removing them from a pack is a decision for a person.
    /// Filled when the source is a bucket and all of it was looked at.
    pub gone_from_source: Vec<String>,
}

/// One run at `now`. `on_pack` is told of each pack as soon as it is written: a run over a
/// whole archive takes hours.
pub fn run(source: &Source, config: &Config, now: Timestamp, on_pack: &dyn Fn(&PackReport)) -> Result<Report> {
    let mut report = Report::default();
    let packed = packed_names(&config.out, &config.also_packed)?;
    let stored = source.list(config.only.as_deref().unwrap_or_default())?;
    if let (Some(only), true) = (&config.only, stored.is_empty()) {
        bail!("no submission's name starts with {only}");
    }

    // What is in a pack and no longer in the source.
    if source.bucket.is_some() && config.only.is_none() {
        let in_source: HashSet<&str> = stored.iter().map(|stored| stored.name.as_str()).collect();
        report.gone_from_source = packed.iter().filter(|name| !in_source.contains(name.as_str())).cloned().collect();
        report.gone_from_source.sort();
    }

    // What is still to pack, by the hour it was received.
    let mut hours: BTreeMap<String, Vec<Stored>> = BTreeMap::new();
    for stored in stored {
        if packed.contains(&stored.name) {
            report.already_packed += 1;
            continue;
        }
        let Some(hour) = received_hour(&stored.name) else {
            report
                .left
                .push(format!("{}: its name does not start with the time it was received", stored.name));
            continue;
        };
        if !has_settled(&hour, config.settle, now) {
            report.not_settled += 1;
            continue;
        }
        hours.entry(hour).or_default().push(stored);
    }

    let budget = BuildBudget::new(config.build_input_bytes);
    for (hour, mut submissions) in hours {
        // A pack holds its submissions in the order they were received.
        submissions.sort_by(|a, b| a.name.cmp(&b.name));
        for batch in batches(submissions, config.pack_input_bytes) {
            if let Some(pack) = pack_hour(source, config, &hour, batch, &budget, &mut report.left)? {
                on_pack(&pack);
                report.packs.push(pack);
            }
        }
    }
    Ok(report)
}

/// The hour a submission was received, as `YYYY/MM/DD/HH`, from its name
/// `<yyyy-mm>/<yyyy-mm-dd>_<hhmmss>Z_<id>.json`. None for a name of another shape.
fn received_hour(name: &str) -> Option<String> {
    let file = name.rsplit('/').next()?;
    let stamp = file.get(..13)?.as_bytes();
    let shaped = stamp.iter().enumerate().all(|(index, byte)| match index {
        4 | 7 => *byte == b'-',
        10 => *byte == b'_',
        _ => byte.is_ascii_digit(),
    });
    shaped.then(|| format!("{}/{}/{}/{}", &file[..4], &file[5..7], &file[8..10], &file[11..13]))
}

/// Whether the hour `YYYY/MM/DD/HH` ended at least `settle` before `now`. An hour that is no
/// time on the calendar never has.
fn has_settled(hour: &str, settle: SignedDuration, now: Timestamp) -> bool {
    let start = format!("{}-{}-{}T{}:00:00Z", &hour[..4], &hour[5..7], &hour[8..10], &hour[11..13]);
    let Ok(start) = start.parse::<Timestamp>() else {
        return false;
    };
    let end = start.checked_add(SignedDuration::from_hours(1)).and_then(|end| end.checked_add(settle));
    end.is_ok_and(|end| end <= now)
}

/// The submissions of one hour, in order, in groups of at most `limit` bytes. One larger
/// than the limit is alone in its group.
fn batches(submissions: Vec<Stored>, limit: u64) -> Vec<Vec<Stored>> {
    let mut batches: Vec<Vec<Stored>> = Vec::new();
    let mut batch_bytes = 0;
    for stored in submissions {
        if batches.is_empty() || batch_bytes + stored.size > limit {
            batches.push(Vec::new());
            batch_bytes = 0;
        }
        batch_bytes += stored.size;
        batches.last_mut().expect("a batch was just started").push(stored);
    }
    batches
}

/// Builds and writes one pack. None when no submission of the batch could be built.
fn pack_hour(source: &Source, config: &Config, hour: &str, batch: Vec<Stored>, budget: &BuildBudget, left: &mut Vec<String>) -> Result<Option<PackReport>> {
    // Build, on every core. A submission that cannot be read or built is left out and
    // reported; it is in no pack afterwards, so a later run meets it again.
    let built = batch
        .par_iter()
        .map_init(Scratch::new, |scratch, stored| {
            let _share = budget.take(stored.size);
            Ok(match source.read(stored)? {
                Ok(file) => build_frame(&stored.name, &file, &config.options, scratch),
                Err(reason) => Err(reason),
            })
        })
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
    for (stored, built) in batch.iter().zip(built) {
        match built {
            Err(reason) => left.push(format!("{}: {reason}", stored.name)),
            Ok(frame) => {
                report.submissions += 1;
                report.kept_whole += frame.entry.exports.iter().any(|(encoding, _)| *encoding == Encoding::Raw) as usize;
                report.value_fallbacks.extend(frame.report.value_fallbacks.iter().cloned());
                report.loose_bytes += frame.report.file_bytes;
                writer.add(frame);
            }
        }
    }
    if writer.is_empty() {
        return Ok(None);
    }
    let pack = writer
        .finish(config.options.level)
        .map_err(|message| anyhow!("the pack of {hour} does not read back: {message}"))?;
    report.key = format!("{PACKS}/{hour}-{}.pack", md5_hex(&pack));
    report.pack_bytes = pack.len() as u64;
    write_whole(&config.out.join(&report.key), &pack)?;
    Ok(Some(report))
}

/// Writes a file so that it is there whole or not at all: under another name first, flushed,
/// then renamed. A file that is there already is a pack with the same checksum, so the same
/// bytes.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().expect("a pack lies in the directory of its day");
    std::fs::create_dir_all(directory).with_context(|| format!("creating {}", directory.display()))?;
    let unfinished = path.with_extension("pack.part");
    let write = || -> std::io::Result<()> {
        let mut file = File::create(&unfinished)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // Closed before it is renamed: a Windows drive may not rename an open file.
        drop(file);
        std::fs::rename(&unfinished, path)?;
        File::open(directory)?.sync_all()
    };
    write().with_context(|| format!("writing {}", path.display()))
}

/// The name of every submission in every pack under `<out>/packs` and `packs/` of each further
/// directory, of every writer. A pack that cannot be read is an error: without it nothing can
/// be said about what is packed. So is a further directory that has no `packs/`, which `out`
/// need not have yet: a mistyped one would have everything packed again.
fn packed_names(out: &Path, also_packed: &[PathBuf]) -> Result<HashSet<String>> {
    let mut packs = Vec::new();
    let root = out.join("packs");
    if root.exists() {
        pack_files(&root, &mut packs)?;
    }
    for directory in also_packed {
        pack_files(&directory.join("packs"), &mut packs)?;
    }
    let names = packs
        .par_iter()
        .map(|path| {
            let mut file = File::open(path)?;
            let length = file.metadata()?.len();
            let fetch = |range: std::ops::Range<u64>| -> Result<Vec<u8>, String> {
                let mut part = vec![0; (range.end - range.start) as usize];
                file.seek(SeekFrom::Start(range.start))
                    .and_then(|_| file.read_exact(&mut part))
                    .map_err(|error| error.to_string())?;
                Ok(part)
            };
            let entries = pack::read_index(length, fetch).map_err(anyhow::Error::msg)?;
            Ok(entries.into_iter().map(|entry| entry.name).collect::<Vec<_>>())
        })
        .zip(&packs)
        .map(|(names, path): (Result<Vec<String>>, _)| names.with_context(|| format!("reading the index of {}", path.display())))
        .collect::<Result<Vec<_>>>()?;
    Ok(names.into_iter().flatten().collect())
}

/// Every `.pack` file under `directory`. A `.pack.part` is a pack a stopped run did not
/// finish, and is no pack.
fn pack_files(directory: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            pack_files(&path, found)?;
        } else if path.extension().is_some_and(|extension| extension == "pack") {
            found.push(path);
        } else if !path.to_string_lossy().ends_with(".pack.part") {
            bail!("{} is not a pack: only packs belong under the output directory's packs/", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_bucket::MemoryBucket;
    use crate::test_support::{now, submission};
    use trace_pack::pack::PackReader;
    use trace_pack::record::Record;

    /// A directory under the system's temporary one, removed when the test ends.
    struct Scratchpad(PathBuf);

    impl Scratchpad {
        fn new(test: &str) -> Self {
            let path = std::env::temp_dir().join(format!("trace-compactor-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Scratchpad(path)
        }

        fn out(&self) -> PathBuf {
            self.0.join("out")
        }

        /// The checkout, with these submissions in it.
        fn checkout(&self, submissions: &[(String, Vec<u8>)]) -> PathBuf {
            let checkout = self.0.join("trace-data");
            for (name, file) in submissions {
                let path = checkout.join(SUBMISSIONS).join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, file).unwrap();
            }
            checkout
        }

        /// Every pack of the output directory, by its path under it, with the names it holds.
        /// Every record must be the file the test gave under that name.
        fn packs(&self, files: &[(String, Vec<u8>)]) -> BTreeMap<String, Vec<String>> {
            let mut found = Vec::new();
            if self.out().join("packs").exists() {
                pack_files(&self.out().join("packs"), &mut found).unwrap();
            }
            let mut packs = BTreeMap::new();
            for path in found {
                let bytes = std::fs::read(&path).unwrap();
                let reader = PackReader::open(&bytes).unwrap();
                let mut names = Vec::new();
                for (index, entry) in reader.entries().iter().enumerate() {
                    let Record::File { name, bytes } = reader.record(index).unwrap() else {
                        panic!("{} was not kept whole", entry.name);
                    };
                    assert_eq!(files.iter().find(|(given, _)| *given == name).map(|(_, file)| file), Some(&bytes), "{name}");
                    names.push(name);
                }
                let key = path.strip_prefix(self.out()).unwrap().to_string_lossy().into_owned();
                assert!(key.ends_with(&format!("-{}.pack", md5_hex(&bytes))), "{key} is not named by its checksum");
                packs.insert(key, names);
            }
            packs
        }
    }

    impl Drop for Scratchpad {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    impl ReadBucket for MemoryBucket {
        fn list(&self, prefix: &str) -> Result<Vec<(String, u64, String)>> {
            Ok(crate::bucket::Bucket::list(self, prefix)?
                .into_iter()
                .map(|object| (object.key, object.size, object.etag))
                .collect())
        }

        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            crate::bucket::Bucket::get(self, key)
        }
    }

    fn config(out: PathBuf) -> Config {
        Config {
            out,
            also_packed: Vec::new(),
            settle: SignedDuration::from_hours(48),
            pack_input_bytes: u64::MAX,
            build_input_bytes: u64::MAX,
            only: None,
            options: PackOptions::default(),
        }
    }

    /// A submission received on 2026-11-08 at `hour`:`minute`.
    fn received(hour: u32, minute: u32, id: &str) -> (String, Vec<u8>) {
        let (name, file) = submission(minute, id);
        (name.replace("_18", &format!("_{hour:02}")), file)
    }

    fn local(checkout: PathBuf) -> Source<'static> {
        Source {
            bucket: None,
            local: Some(checkout),
        }
    }

    #[test]
    fn a_checkout_is_packed_by_the_hour_received_and_a_second_run_packs_nothing() {
        let scratch = Scratchpad::new("by-hour");
        let files = [received(18, 5, "aa"), received(18, 40, "bb"), received(19, 1, "cc")];
        let source = local(scratch.checkout(&files));

        let report = run(&source, &config(scratch.out()), now(), &|_| {}).unwrap();

        let packs = scratch.packs(&files);
        let hours: Vec<(&str, &Vec<String>)> = packs.iter().map(|(key, names)| (&key[..key.rfind('-').unwrap()], names)).collect();
        assert_eq!(
            hours,
            [
                ("packs/backfill/2026/11/08/18", &vec![files[0].0.clone(), files[1].0.clone()]),
                ("packs/backfill/2026/11/08/19", &vec![files[2].0.clone()]),
            ]
        );
        assert_eq!(report.packs.iter().map(|pack| pack.submissions).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(
            report.packs.iter().map(|pack| pack.key.as_str()).collect::<Vec<_>>(),
            packs.keys().map(String::as_str).collect::<Vec<_>>()
        );
        // The checkout is only read.
        assert_eq!(list_submissions(source.local.as_ref().unwrap()).unwrap().len(), 3);

        let again = run(&source, &config(scratch.out()), now(), &|_| {}).unwrap();
        assert_eq!((again.packs.len(), again.already_packed), (0, 3));
        assert_eq!(scratch.packs(&files), packs);
    }

    #[test]
    fn a_submission_that_arrives_after_its_hour_was_packed_gets_a_pack_of_its_own() {
        let scratch = Scratchpad::new("late");
        let mut files = vec![received(18, 5, "aa"), received(18, 40, "bb")];
        run(&local(scratch.checkout(&files)), &config(scratch.out()), now(), &|_| {}).unwrap();

        // Accepted days after it was received: it appears under the hour that is packed.
        files.push(received(18, 20, "late"));
        let report = run(&local(scratch.checkout(&files)), &config(scratch.out()), now(), &|_| {}).unwrap();

        assert_eq!((report.packs.len(), report.packs[0].submissions, report.already_packed), (1, 1, 2));
        let packs = scratch.packs(&files);
        assert_eq!(packs.len(), 2);
        assert!(packs.keys().all(|key| key.starts_with("packs/backfill/2026/11/08/18-")));
        assert_eq!(packs[&report.packs[0].key], [files[2].0.clone()]);
    }

    #[test]
    fn an_hour_is_packed_only_once_it_has_settled() {
        let scratch = Scratchpad::new("settle");
        let files = [received(18, 5, "aa"), received(19, 5, "bb")];
        let source = local(scratch.checkout(&files));
        // Two days after the end of hour 18, to the second: hour 19 has an hour to go.
        let at: Timestamp = "2026-11-10T19:00:00Z".parse().unwrap();

        let a_second_early = run(&source, &config(scratch.out()), at.checked_sub(SignedDuration::from_secs(1)).unwrap(), &|_| {}).unwrap();
        assert_eq!((a_second_early.packs.len(), a_second_early.not_settled), (0, 2));

        let report = run(&source, &config(scratch.out()), at, &|_| {}).unwrap();
        assert_eq!((report.packs.len(), report.not_settled), (1, 1));
        assert_eq!(scratch.packs(&files).into_values().collect::<Vec<_>>(), [vec![files[0].0.clone()]]);
    }

    #[test]
    fn the_size_limit_splits_an_hour_and_only_narrows_what_is_looked_at() {
        let scratch = Scratchpad::new("limit");
        let files = [received(18, 1, "aa"), received(18, 2, "bb"), received(18, 3, "cc"), received(19, 1, "dd")];
        let source = local(scratch.checkout(&files));
        let limited = Config {
            pack_input_bytes: 2 * files[0].1.len() as u64,
            only: Some("2026-11/2026-11-08_18".into()),
            ..config(scratch.out())
        };

        let report = run(&source, &limited, now(), &|_| {}).unwrap();

        // Hour 18 in two packs, in the order received. Hour 19 was not asked for.
        assert_eq!(report.packs.iter().map(|pack| pack.submissions).collect::<Vec<_>>(), [2, 1]);
        let mut packed: Vec<Vec<String>> = scratch.packs(&files).into_values().collect();
        packed.sort();
        assert_eq!(packed, [vec![files[0].0.clone(), files[1].0.clone()], vec![files[2].0.clone()]]);
    }

    #[test]
    fn a_file_that_is_not_named_like_a_submission_is_reported_and_left() {
        let scratch = Scratchpad::new("odd-name");
        let files = [received(18, 5, "aa"), ("2026-11/notes.json".to_owned(), b"{}".to_vec())];
        let report = run(&local(scratch.checkout(&files)), &config(scratch.out()), now(), &|_| {}).unwrap();
        assert_eq!(report.left, ["2026-11/notes.json: its name does not start with the time it was received"]);
        assert_eq!(scratch.packs(&files).into_values().collect::<Vec<_>>(), [vec![files[0].0.clone()]]);
    }

    /// With a bucket, the bucket says what exists and what each object is. A local file
    /// saves the download only when it is that object.
    #[test]
    fn a_local_file_is_used_only_when_it_is_the_object_the_bucket_lists() {
        let scratch = Scratchpad::new("layered");
        let files = [
            received(18, 1, "same"),
            received(18, 2, "stale"),
            received(18, 3, "remote"),
            received(18, 4, "local-only"),
        ];
        let bucket = MemoryBucket::new(now());
        for (name, file) in &files[..3] {
            bucket.store(&format!("{SUBMISSIONS}{name}"), file, now());
        }
        bucket.store(SUBMISSIONS, b"", now());
        let checkout = scratch.checkout(&[
            files[0].clone(),
            // An older download of an object that was replaced since.
            (files[1].0.clone(), b"what this object was before".to_vec()),
            // Rejected and deleted from the bucket after it was downloaded.
            files[3].clone(),
        ]);
        let source = Source {
            bucket: Some(&bucket),
            local: Some(checkout),
        };

        let report = run(&source, &config(scratch.out()), now(), &|_| {}).unwrap();

        // The three the bucket lists, each with the bucket's bytes; `packs` compares them.
        assert_eq!(
            scratch.packs(&files).into_values().collect::<Vec<_>>(),
            [vec![files[0].0.clone(), files[1].0.clone(), files[2].0.clone()]]
        );
        assert_eq!(report.left, [] as [String; 0]);
        // Two downloads: the one that was not on disk and the one that differed.
        assert_eq!(bucket.gets(), 2);
    }

    /// The old bucket is in production and holds several hundred thousand objects: a run over
    /// one day must not list all of it.
    #[test]
    fn a_narrowed_run_asks_the_bucket_for_that_part_only() {
        let scratch = Scratchpad::new("narrowed");
        let files = [received(18, 1, "aa"), received(19, 1, "bb")];
        let bucket = MemoryBucket::new(now());
        for (name, file) in &files {
            bucket.store(&format!("{SUBMISSIONS}{name}"), file, now());
        }
        struct OnlyHour18<'a>(&'a MemoryBucket);
        impl ReadBucket for OnlyHour18<'_> {
            fn list(&self, prefix: &str) -> Result<Vec<(String, u64, String)>> {
                assert_eq!(prefix, "submissions/2026-11/2026-11-08_18");
                ReadBucket::list(self.0, prefix)
            }
            fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
                ReadBucket::get(self.0, key)
            }
        }
        let source = Source {
            bucket: Some(&OnlyHour18(&bucket)),
            local: None,
        };
        let narrowed = |only: &str| Config {
            only: Some(only.into()),
            ..config(scratch.out())
        };
        // Hour 19 is packed already, by a run that looked at all of it.
        let everything = Source {
            bucket: None,
            local: Some(scratch.checkout(&files[1..])),
        };
        run(&everything, &config(scratch.out()), now(), &|_| {}).unwrap();

        let report = run(&source, &narrowed("2026-11/2026-11-08_18"), now(), &|_| {}).unwrap();

        assert_eq!(report.packs.len(), 1);
        assert_eq!(scratch.packs(&files).len(), 2);
        // Hour 19 is in a pack and was not listed. That is not gone, only not looked at.
        assert_eq!(report.gone_from_source, [] as [String; 0]);

        // A mistyped day lists nothing, and must not pass for a day that is done.
        let nothing = Source {
            bucket: Some(&bucket),
            local: None,
        };
        run(&nothing, &narrowed("2026-11/2026-11-80"), now(), &|_| {}).unwrap_err();
    }

    /// The run is left alone for hours: a bucket that stops answering ends it with whole packs,
    /// and the next run packs the rest and nothing twice.
    #[test]
    fn a_run_whose_bucket_stops_answering_is_continued_by_the_next() {
        let scratch = Scratchpad::new("stops");
        let files = [received(18, 1, "aa"), received(19, 1, "bb"), received(20, 1, "cc")];
        let bucket = MemoryBucket::new(now());
        for (name, file) in &files {
            bucket.store(&format!("{SUBMISSIONS}{name}"), file, now());
        }
        let source = Source {
            bucket: Some(&bucket),
            local: None,
        };
        // The listing, the first hour's download, and then nothing more.
        bucket.fail_call(2, crate::memory_bucket::Fault::Refused);
        run(&source, &config(scratch.out()), now(), &|_| {}).unwrap_err();
        assert!(bucket.restore());
        assert_eq!(scratch.packs(&files).into_values().collect::<Vec<_>>(), [vec![files[0].0.clone()]]);

        let report = run(&source, &config(scratch.out()), now(), &|_| {}).unwrap();

        assert_eq!((report.already_packed, report.packs.len()), (1, 2));
        let packed: Vec<Vec<String>> = scratch.packs(&files).into_values().collect();
        assert_eq!(packed, files.iter().map(|(name, _)| vec![name.clone()]).collect::<Vec<_>>());
    }

    /// A pack is written under another name first. Where that name cannot be written, nothing
    /// may appear under the pack's own: a later run would take it for a whole pack.
    #[test]
    fn a_pack_that_cannot_be_written_whole_does_not_appear() {
        let (scratch, files) = (Scratchpad::new("unwritable"), [received(18, 5, "aa")]);
        let source = local(scratch.checkout(&files));
        let elsewhere = scratch.0.join("elsewhere");
        let key = run(&source, &config(elsewhere), now(), &|_| {}).unwrap().packs[0].key.clone();
        std::fs::create_dir_all(scratch.out().join(&key).with_extension("pack.part")).unwrap();

        run(&source, &config(scratch.out()), now(), &|_| {}).unwrap_err();

        assert!(!scratch.out().join(&key).exists());
    }

    /// An archive on two disks: what is packed on one is not packed again on the other.
    #[test]
    fn packs_in_a_further_directory_count_as_packed_and_a_directory_without_packs_is_an_error() {
        let scratch = Scratchpad::new("two-disks");
        let files = [received(18, 1, "aa"), received(18, 2, "bb"), received(19, 1, "cc")];
        let first_disk = scratch.0.join("first-disk");
        run(&local(scratch.checkout(&files[..2])), &config(first_disk.clone()), now(), &|_| {}).unwrap();
        let before = std::fs::read_dir(first_disk.join("packs/backfill/2026/11/08")).unwrap().count();
        let second_disk = Config {
            also_packed: vec![first_disk.clone()],
            ..config(scratch.out())
        };

        let report = run(&local(scratch.checkout(&files)), &second_disk, now(), &|_| {}).unwrap();

        assert_eq!((report.already_packed, report.packs.len()), (2, 1));
        assert_eq!(scratch.packs(&files).into_values().collect::<Vec<_>>(), [vec![files[2].0.clone()]]);
        assert_eq!(std::fs::read_dir(first_disk.join("packs/backfill/2026/11/08")).unwrap().count(), before);

        // A mistyped directory holds no packs. Taking that for "nothing is packed" would pack
        // the whole archive again.
        let mistyped = Config {
            also_packed: vec![scratch.0.join("frist-disk")],
            ..config(scratch.out())
        };
        run(&local(scratch.checkout(&files)), &mistyped, now(), &|_| {}).unwrap_err();
    }

    #[test]
    fn what_is_packed_and_gone_from_the_bucket_is_reported_and_stays_packed() {
        let scratch = Scratchpad::new("gone");
        let files = [received(18, 1, "kept"), received(18, 2, "rejected-since")];
        let bucket = MemoryBucket::new(now());
        for (name, file) in &files {
            bucket.store(&format!("{SUBMISSIONS}{name}"), file, now());
        }
        let source = Source {
            bucket: Some(&bucket),
            local: None,
        };
        run(&source, &config(scratch.out()), now(), &|_| {}).unwrap();

        bucket.remove(&format!("{SUBMISSIONS}{}", files[1].0));
        let report = run(&source, &config(scratch.out()), now(), &|_| {}).unwrap();

        assert_eq!(report.gone_from_source, [files[1].0.clone()]);
        assert_eq!(
            scratch.packs(&files).into_values().collect::<Vec<_>>(),
            [vec![files[0].0.clone(), files[1].0.clone()]]
        );
        // The source was only read, in both runs.
        assert_eq!(bucket.keys(""), [format!("{SUBMISSIONS}{}", files[0].0)]);
    }

    #[test]
    fn an_object_that_is_not_what_the_bucket_listed_is_left_and_the_rest_is_packed() {
        let scratch = Scratchpad::new("changed");
        let files = [received(18, 1, "aa"), received(18, 2, "bb")];
        let bucket = MemoryBucket::new(now());
        for (name, file) in &files {
            bucket.store(&format!("{SUBMISSIONS}{name}"), file, now());
        }
        let changed = format!("{SUBMISSIONS}{}", files[0].0);
        bucket.after_listing(SUBMISSIONS, &changed, b"replaced after the listing");
        let source = Source {
            bucket: Some(&bucket),
            local: None,
        };

        let report = run(&source, &config(scratch.out()), now(), &|_| {}).unwrap();

        assert!(
            report.left.len() == 1 && report.left[0].starts_with(&files[0].0) && report.left[0].contains("not the MD5"),
            "{:?}",
            report.left
        );
        assert_eq!(scratch.packs(&files).into_values().collect::<Vec<_>>(), [vec![files[1].0.clone()]]);
    }

    #[test]
    fn a_run_that_was_stopped_mid_write_leaves_nothing_a_later_run_takes_for_a_pack() {
        let scratch = Scratchpad::new("part");
        let files = [received(18, 5, "aa")];
        let source = local(scratch.checkout(&files));
        let day = scratch.out().join("packs/backfill/2026/11/08");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("18-0000.pack.part"), b"half a pack").unwrap();

        let report = run(&source, &config(scratch.out()), now(), &|_| {}).unwrap();
        assert_eq!(report.packs.len(), 1);

        // Anything else under packs/ stops the run: it cannot say what that file holds.
        std::fs::write(day.join("18-0000.pack"), b"not a pack").unwrap();
        run(&source, &config(scratch.out()), now(), &|_| {}).unwrap_err();
        std::fs::remove_file(day.join("18-0000.pack")).unwrap();
        std::fs::write(day.join("notes.txt"), b"x").unwrap();
        run(&source, &config(scratch.out()), now(), &|_| {}).unwrap_err();
    }
}
