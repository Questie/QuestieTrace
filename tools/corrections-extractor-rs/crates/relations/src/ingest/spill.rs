//! Ingest's intermediate files, so it never holds every session at once.
//!
//! Ingest works in three passes (see [`super::run`]):
//! 1. Decoding writes every session copy to a partition chosen by its session key, so all copies
//!    of a session meet in one partition.
//! 2. Each copy partition is merged on its own. The sessions kept go to a partition chosen by the
//!    first two bytes of their contributor id: every session of a contributor meets in one
//!    partition, and partitions read in ascending order give the contributors in sorted order.
//! 3. Each contributor partition becomes episodes, written straight to the episode store.
//!
//! The files use the episode store's binary [`codec`](crate::episode_store::codec): each record
//! is a varint byte length, then the record. They live in a directory removed when ingest ends.

use super::distill::{DistillStats, DistilledSession, EpisodeDraft, PendingGreeting, TitleObservation};
use crate::episode_store::codec::{codec_struct, read_varint, write_varint, Decode, Encode, Input};
use anyhow::{bail, Context, Result};
use rustc_hash::FxHasher;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::hash::Hasher;
use std::io::{BufWriter, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

codec_struct!(DistillStats {
    detail_unresolved,
    progress_unresolved,
    greeting_without_count,
    gossip_without_read,
    item_started_details,
    borrowed_quest_ids,
    gossip_inherited_from_other_giver,
    completed_read_gaps,
});
codec_struct!(TitleObservation { title, quest_id, giver });
codec_struct!(PendingGreeting {
    offer,
    available,
    active,
    expected,
    incomplete,
});
codec_struct!(EpisodeDraft {
    session_name,
    interface_version,
    locale,
    player,
    duration,
    levels,
    completed,
    quest_log,
    quest_events,
    offers,
    quest_info,
});
codec_struct!(DistilledSession {
    key,
    event_count,
    saved,
    start_server_time,
    episode,
    pending_greetings,
    titles,
    stats,
});

/// One session copy as decoded, with the submission it came from.
pub struct SpilledCopy {
    pub submission_id: String,
    pub contributor_id: String,
    pub received_at: String,
    /// The submission's position in path order.
    pub index: usize,
    /// The copy's position among the submission's sessions.
    pub position: usize,
    pub session: DistilledSession,
}

codec_struct!(SpilledCopy {
    submission_id,
    contributor_id,
    received_at,
    index,
    position,
    session,
});

/// A merged session ingest keeps, with what turning it into an episode needs.
pub struct KeptSession {
    pub key: String,
    /// Contributor of the earliest received submission containing the session.
    pub contributor_id: String,
    pub first_received_at: String,
    /// Ascending.
    pub submission_ids: Vec<String>,
    /// GetServerTime at capture start; orders a contributor's sessions.
    pub start_server_time: Option<f64>,
    pub episode: EpisodeDraft,
}

codec_struct!(KeptSession {
    key,
    contributor_id,
    first_received_at,
    submission_ids,
    start_server_time,
    episode,
});

/// How many partitions session copies are spread over.
pub const COPY_PARTITIONS: u32 = 64;

/// The copy partition of a session: a hash of its key, the same in every run.
pub fn copy_partition(session_key: &str) -> u32 {
    let mut hasher = FxHasher::default();
    hasher.write(session_key.as_bytes());
    (hasher.finish() % u64::from(COPY_PARTITIONS)) as u32
}

/// The contributor partition of a session: its contributor id's first two bytes. Comparing
/// partitions compares the ids' first two bytes, as sorting the ids as text does, so partitions in
/// ascending order hold the contributors in sorted order.
pub fn contributor_partition(contributor_id: &str) -> u32 {
    let bytes = contributor_id.as_bytes();
    let byte = |at: usize| u32::from(bytes.get(at).copied().unwrap_or(0));
    (byte(0) << 8) | byte(1)
}

/// Records of one kind in numbered partition files under one directory.
pub struct Partitions<T> {
    dir: PathBuf,
    files: BTreeMap<u32, BufWriter<File>>,
    record: Vec<u8>,
    kind: PhantomData<T>,
}

impl<T: Encode + Decode> Partitions<T> {
    /// An empty set of partitions in `dir`, which is replaced if it exists.
    pub fn create(dir: &Path) -> Result<Partitions<T>> {
        if dir.exists() {
            fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Partitions {
            dir: dir.to_owned(),
            files: BTreeMap::new(),
            record: Vec::new(),
            kind: PhantomData,
        })
    }

    fn path(dir: &Path, partition: u32) -> PathBuf {
        dir.join(format!("{partition:05}.bin"))
    }

    pub fn push(&mut self, partition: u32, value: &T) -> Result<()> {
        if !self.files.contains_key(&partition) {
            let path = Self::path(&self.dir, partition);
            let file = File::create(&path).with_context(|| format!("creating {}", path.display()))?;
            self.files.insert(partition, BufWriter::with_capacity(1 << 16, file));
        }
        let out = self.files.get_mut(&partition).expect("opened above");
        self.record.clear();
        value.encode(&mut self.record);
        let mut length = Vec::with_capacity(10);
        write_varint(&mut length, self.record.len() as u64);
        out.write_all(&length)
            .and_then(|()| out.write_all(&self.record))
            .with_context(|| format!("writing {}", Self::path(&self.dir, partition).display()))
    }

    /// Closes every file; the partitions can be read from then on.
    pub fn finish(self) -> Result<Written<T>> {
        let mut partitions = Vec::with_capacity(self.files.len());
        for (partition, mut file) in self.files {
            file.flush()
                .with_context(|| format!("writing {}", Self::path(&self.dir, partition).display()))?;
            partitions.push(partition);
        }
        Ok(Written {
            dir: self.dir,
            partitions,
            kind: PhantomData,
        })
    }
}

/// Partitions written and closed.
pub struct Written<T> {
    dir: PathBuf,
    /// The partitions holding records, ascending.
    pub partitions: Vec<u32>,
    kind: PhantomData<T>,
}

impl<T: Encode + Decode> Written<T> {
    /// Every record of `partition`, in the order written.
    pub fn read(&self, partition: u32) -> Result<Vec<T>> {
        let path = Partitions::<T>::path(&self.dir, partition);
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut input = Input::new(&bytes);
        let mut records = Vec::new();
        while !input.is_empty() {
            let length = usize::try_from(read_varint(&mut input)?).context("record length out of range")?;
            let mut record = Input::new(input.take(length)?);
            records.push(T::decode(&mut record).with_context(|| format!("{} record {}", path.display(), records.len()))?);
            if !record.is_empty() {
                bail!("{} record {} has bytes left over", path.display(), records.len() - 1);
            }
        }
        Ok(records)
    }

    /// Removes the partition files.
    pub fn remove(self) -> Result<()> {
        fs::remove_dir_all(&self.dir).with_context(|| format!("removing {}", self.dir.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contributor_partitions_sort_like_the_ids_they_hold() {
        let mut ids = vec!["ff", "0a3", "a", "a0", "9f", "", "0", "b7c", "a00"];
        ids.sort();
        let partitions: Vec<u32> = ids.iter().map(|id| contributor_partition(id)).collect();
        assert!(partitions.windows(2).all(|pair| pair[0] <= pair[1]), "{ids:?} -> {partitions:?}");
        assert_eq!(contributor_partition("a0"), contributor_partition("a00"), "two bytes decide");
    }

    #[test]
    fn copy_partitions_are_stable_and_in_range() {
        assert_eq!(copy_partition("1790813100.5:1790813100.25"), copy_partition("1790813100.5:1790813100.25"));
        assert!((0..1000).all(|n| copy_partition(&format!("session {n}")) < COPY_PARTITIONS));
    }

    #[test]
    fn records_read_back_per_partition_in_the_order_written() {
        let dir = std::env::temp_dir().join(format!("ingest-spill-test-{}", std::process::id()));
        let mut partitions = Partitions::<Vec<String>>::create(&dir).unwrap();
        for (partition, text) in [(3, "a"), (1, "b"), (3, "c"), (1, ""), (7, "d")] {
            partitions.push(partition, &vec![text.to_owned()]).unwrap();
        }
        let written = partitions.finish().unwrap();
        assert_eq!(written.partitions, [1, 3, 7]);
        assert_eq!(written.read(3).unwrap(), [vec!["a".to_owned()], vec!["c".to_owned()]]);
        assert_eq!(written.read(1).unwrap(), [vec!["b".to_owned()], vec![String::new()]]);
        written.remove().unwrap();
        assert!(!dir.exists());
    }
}
