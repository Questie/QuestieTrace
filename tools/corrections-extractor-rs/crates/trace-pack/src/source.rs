//! Where a tool reads submissions from: a trace-data checkout or a directory of packs.
//!
//! Both give the same thing: every submission under its key, `<yyyy-mm>/<file>.json`, and
//! for each the [`Submission`] its file holds. A tool that lists and reads through this
//! module does not know which of the two it was given, so every rule it applies afterwards,
//! such as which sessions it leaves out, applies to both alike.

use crate::pack::{self, record_of};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use trace_data::source::list_submissions;
use trace_data::submission::{submission_id_from_path, Submission};
use trace_data::Inflater;

/// One submission, wherever it is stored.
#[derive(Clone, Debug)]
pub struct StoredSubmission {
    /// `<yyyy-mm>/<file>.json`: the name of the submission's file, also for one in a pack.
    pub key: String,
    location: Location,
}

#[derive(Clone, Debug)]
enum Location {
    File(PathBuf),
    /// One frame of a pack file.
    Frame {
        pack: Arc<PathBuf>,
        offset: u64,
        length: u64,
    },
}

/// Every submission under `directory`, newest first: keys start with the time received.
///
/// `directory` is a trace-data checkout, with `submissions/<yyyy-mm>/*.json`, or holds
/// `packs/`, with `.pack` files at any depth. A submission that is in two packs, which the
/// packing allows, is listed once.
pub fn list(directory: &Path) -> io::Result<Vec<StoredSubmission>> {
    let (checkout, packs) = (directory.join("submissions").is_dir(), directory.join("packs").is_dir());
    if checkout && packs {
        let message = format!(
            "{} holds both submissions/ and packs/: which to read is not for a tool to guess",
            directory.display()
        );
        return Err(io::Error::new(io::ErrorKind::InvalidInput, message));
    }
    if packs {
        return list_packs(&directory.join("packs"));
    }
    let files = list_submissions(directory)?;
    Ok(files.into_iter().map(|file| StoredSubmission::file(file.key, file.path)).collect())
}

/// Every submission of every pack under `packs`. A pack whose index cannot be read is an
/// error: a listing is all of the submissions or none.
fn list_packs(packs: &Path) -> io::Result<Vec<StoredSubmission>> {
    let mut paths = Vec::new();
    pack_files(packs, &mut paths)?;
    paths.sort();
    let listed = paths
        .par_iter()
        .map(|path| {
            let unreadable = |message: String| io::Error::other(format!("{}: {message}", path.display()));
            let mut file = File::open(path)?;
            let length = file.metadata()?.len();
            let fetch = |range: std::ops::Range<u64>| read_range(&mut file, range).map_err(|error| error.to_string());
            let entries = pack::read_index(length, fetch).map_err(unreadable)?;
            let pack = Arc::new(path.clone());
            Ok(entries
                .into_iter()
                .map(|entry| StoredSubmission {
                    key: entry.name,
                    location: Location::Frame {
                        pack: pack.clone(),
                        offset: entry.offset,
                        length: entry.length,
                    },
                })
                .collect::<Vec<_>>())
        })
        .collect::<io::Result<Vec<_>>>()?;

    // The first copy of each, in the order of the pack files' paths.
    let mut seen = HashSet::new();
    let mut submissions: Vec<StoredSubmission> = listed.into_iter().flatten().filter(|stored| seen.insert(stored.key.clone())).collect();
    submissions.sort_by(|a, b| b.key.cmp(&a.key));
    Ok(submissions)
}

fn pack_files(directory: &Path, found: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            pack_files(&path, found)?;
        } else if path.extension().is_some_and(|extension| extension == "pack") {
            found.push(path);
        }
    }
    Ok(())
}

fn read_range(file: &mut File, range: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; (range.end - range.start) as usize];
    file.seek(SeekFrom::Start(range.start))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

impl StoredSubmission {
    /// A submission that is a file of its own.
    pub fn file(key: String, path: PathBuf) -> Self {
        StoredSubmission {
            key,
            location: Location::File(path),
        }
    }

    /// The id in the submission's name: what a reader falls back on when the file names none.
    pub fn id_in_name(&self) -> String {
        submission_id_from_path(Path::new(&self.key))
    }

    /// Reads and inflates the submission. Never fails: what went wrong is kept in the
    /// submission, as [`Submission::read`] keeps it for a file, and a tool reports it as it
    /// reports a file it could not read.
    pub fn read(&self, inflater: &mut Inflater) -> Submission {
        match &self.location {
            Location::File(path) => Submission::read(path, inflater),
            Location::Frame { pack, offset, length } => {
                let mut read = || -> Result<Submission, String> {
                    let mut file = File::open(pack.as_path()).map_err(|error| error.to_string())?;
                    let frame = read_range(&mut file, *offset..*offset + *length).map_err(|error| error.to_string())?;
                    record_of(&frame, &self.key)?.submission(inflater)
                };
                read().unwrap_or_else(|message| Submission::unreadable(self.id_in_name(), format!("unreadable in {}: {message}", pack.display())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::{export_string, file, options, pack_of, read, trace};
    use crate::pack::PackReader;

    /// A directory under the system's temporary one, removed when the test ends.
    struct Scratchpad(PathBuf);

    impl Scratchpad {
        fn new(test: &str) -> Self {
            let path = std::env::temp_dir().join(format!("trace-pack-source-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Scratchpad(path)
        }

        fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Scratchpad {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Three submissions: two with a trace, of which one rounds `tp`, and one whose export
    /// does not inflate.
    fn submissions() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("2026-11/2026-11-08_180000Z_aaa.json", file(&export_string(&trace(7.370141)))),
            ("2026-11/2026-11-08_190000Z_bbb.json", file(&export_string(&trace(1.25)))),
            ("2026-12/2026-12-01_000000Z_ccc.json", file("!QuestieTrace:1!broken!End:QuestieTrace:1!")),
        ]
    }

    /// The same submissions as a checkout and as packs: two packs, in neither key order nor
    /// one pack per month.
    fn both(scratch: &Scratchpad) -> (PathBuf, PathBuf) {
        let files = submissions();
        for (name, bytes) in &files {
            scratch.write(&format!("checkout/submissions/{name}"), bytes);
        }
        let (first, _) = pack_of(&[files[2].clone(), files[0].clone()], &options());
        let (second, _) = pack_of(&[files[1].clone()], &options());
        scratch.write("packed/packs/backfill/2026/12/01/00-x.pack", &first);
        scratch.write("packed/packs/live/2026/11/10/19-y.pack", &second);
        (scratch.0.join("checkout"), scratch.0.join("packed"))
    }

    #[test]
    fn packs_list_and_read_as_the_checkout_they_were_made_from() {
        let scratch = Scratchpad::new("same");
        let (checkout, packed) = both(&scratch);

        let (from_files, from_packs) = (list(&checkout).unwrap(), list(&packed).unwrap());

        // The same keys in the same order, newest first, whatever pack each is in.
        let keys = |listed: &[StoredSubmission]| listed.iter().map(|stored| stored.key.clone()).collect::<Vec<_>>();
        assert_eq!(keys(&from_packs), keys(&from_files));
        assert_eq!(
            keys(&from_packs),
            [
                "2026-12/2026-12-01_000000Z_ccc.json",
                "2026-11/2026-11-08_190000Z_bbb.json",
                "2026-11/2026-11-08_180000Z_aaa.json"
            ]
        );

        // And for each the same ids, errors and traces.
        let mut inflater = Inflater::new();
        for (packed, filed) in from_packs.iter().zip(&from_files) {
            assert_eq!(read(&packed.read(&mut inflater)), read(&filed.read(&mut inflater)), "{}", packed.key);
        }
        // Of which one decodes to a trace and one fails as its file does.
        assert_eq!(read(&from_packs[2].read(&mut inflater)).5.len(), 1);
        assert!(read(&from_packs[0].read(&mut inflater)).3.is_some() || !read(&from_packs[0].read(&mut inflater)).4.is_empty());
    }

    #[test]
    fn a_submission_in_two_packs_is_listed_once() {
        let scratch = Scratchpad::new("twice");
        let files = submissions();
        let (both, _) = pack_of(&files[..2], &options());
        let (again, _) = pack_of(&files[1..2], &options());
        scratch.write("packs/live/a.pack", &both);
        scratch.write("packs/live/b.pack", &again);

        let listed = list(&scratch.0).unwrap();

        assert_eq!(listed.iter().map(|stored| stored.key.as_str()).collect::<Vec<_>>(), [files[1].0, files[0].0]);
    }

    #[test]
    fn a_damaged_frame_is_one_unreadable_submission_that_names_its_pack() {
        let scratch = Scratchpad::new("damaged");
        let files = submissions();
        let (mut pack, _) = pack_of(&files[..2], &options());
        let first = PackReader::open(&pack).unwrap().entries()[0].clone();
        pack[(first.offset + first.length / 2) as usize] ^= 1;
        scratch.write("packs/live/a.pack", &pack);

        let listed = list(&scratch.0).unwrap();
        let mut inflater = Inflater::new();
        let damaged = listed.iter().find(|stored| stored.key == first.name).unwrap().read(&mut inflater);
        let other = listed.iter().find(|stored| stored.key != first.name).unwrap().read(&mut inflater);

        let (id, _, _, error, _, traces) = read(&damaged);
        assert_eq!((id.as_str(), traces.len()), ("aaa", 0));
        assert!(error.as_ref().is_some_and(|error| error.contains("a.pack")), "{error:?}");
        assert_eq!(read(&other).5.len(), 1);
    }

    #[test]
    fn a_directory_that_is_not_one_of_the_two_is_refused() {
        let scratch = Scratchpad::new("neither");
        assert!(list(&scratch.0).is_err());

        // Both: which one a tool reads must not depend on which it looks for first.
        let (checkout, _) = both(&scratch);
        scratch.write("checkout/packs/live/a.pack", b"x");
        assert_eq!(list(&checkout).unwrap_err().kind(), io::ErrorKind::InvalidInput);

        // A pack that does not open fails the listing: part of an archive is not the archive.
        let scratch = Scratchpad::new("bad-pack");
        scratch.write("packs/live/a.pack", b"not a pack");
        assert!(list(&scratch.0).unwrap_err().to_string().contains("a.pack"));
    }
}
