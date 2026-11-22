//! The episodes on disk, in buckets of whole characters, read back one bucket at a time.
//!
//! Every episode at once took 15 GB on the archive of November 11 and grows with it. Every step
//! that reads episodes works one character at a time and adds up across characters, so it only
//! needs one bucket in memory: about [`BUCKET_BYTES`] of encoded episodes, some 130 MB decoded.
//!
//! Layout of the store directory:
//! - `index.json`: the format version, the counts, and per bucket its file and counts.
//! - `00000.bin`, `00001.bin`, ...: the buckets. Each holds episodes as a varint byte length
//!   followed by the episode in the [`codec`] format.
//!
//! Episodes keep ingest's order: by contributor, then character, then session order. A bucket
//! only ends where a character ends, so each character is whole in one bucket. Reading the
//! buckets in order reads every episode in that order.

pub mod codec;

use crate::model::QuestEpisode;
use anyhow::{bail, Context, Result};
use codec::{read_varint, write_varint, Decode, Encode, Input};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// The layout of the bucket files; a store written in another layout is refused.
pub const FORMAT_VERSION: u32 = 1;

/// By default a bucket ends at the first character boundary after this many bytes.
pub const BUCKET_BYTES: usize = 32 << 20;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    format: u32,
    episodes: usize,
    characters: usize,
    buckets: Vec<Bucket>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bucket {
    file: String,
    episodes: usize,
    characters: usize,
    bytes: usize,
}

/// Episodes as the steps read them: one bucket of whole characters at a time, in ingest's order.
pub trait Episodes: Sync {
    /// Hands every bucket to `work`, in order; stops at the first error.
    fn for_each_bucket(&self, work: &mut dyn FnMut(&[QuestEpisode]) -> Result<()>) -> Result<()>;

    fn episode_count(&self) -> usize;
}

/// Episodes already in memory, in ingest's order, as one bucket (tests and small inputs).
pub struct InMemory<'e>(pub &'e [QuestEpisode]);

impl Episodes for InMemory<'_> {
    fn for_each_bucket(&self, work: &mut dyn FnMut(&[QuestEpisode]) -> Result<()>) -> Result<()> {
        work(self.0)
    }

    fn episode_count(&self) -> usize {
        self.0.len()
    }
}

/// Episodes in memory with every character in a bucket of its own: for tests that a step reading
/// bucket by bucket finds what it finds with every episode in one bucket.
#[cfg(test)]
pub struct PerCharacter<'e>(pub &'e [QuestEpisode]);

#[cfg(test)]
impl Episodes for PerCharacter<'_> {
    fn for_each_bucket(&self, work: &mut dyn FnMut(&[QuestEpisode]) -> Result<()>) -> Result<()> {
        let same_character = |a: &QuestEpisode, b: &QuestEpisode| a.contributor_id == b.contributor_id && a.character_key == b.character_key;
        for character in self.0.chunk_by(same_character) {
            work(character)?;
        }
        Ok(())
    }

    fn episode_count(&self) -> usize {
        self.0.len()
    }
}

impl Episodes for EpisodeStore {
    fn for_each_bucket(&self, work: &mut dyn FnMut(&[QuestEpisode]) -> Result<()>) -> Result<()> {
        EpisodeStore::for_each_bucket(self, work)
    }

    fn episode_count(&self) -> usize {
        EpisodeStore::episode_count(self)
    }
}

/// A written store, ready to read.
pub struct EpisodeStore {
    dir: PathBuf,
    index: Index,
}

impl EpisodeStore {
    pub fn open(dir: &Path) -> Result<EpisodeStore> {
        let path = dir.join("index.json");
        let text = fs::read_to_string(&path).with_context(|| format!("{} is missing: run `relations ingest` first", path.display()))?;
        let index: Index = serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
        if index.format != FORMAT_VERSION {
            bail!(
                "{} holds format {}, this build reads format {FORMAT_VERSION}: run `relations ingest` again",
                dir.display(),
                index.format
            );
        }
        Ok(EpisodeStore { dir: dir.to_owned(), index })
    }

    pub fn episode_count(&self) -> usize {
        self.index.episodes
    }

    pub fn character_count(&self) -> usize {
        self.index.characters
    }

    pub fn bucket_count(&self) -> usize {
        self.index.buckets.len()
    }

    /// The episodes of bucket `number`, in order.
    pub fn load_bucket(&self, number: usize) -> Result<Vec<QuestEpisode>> {
        let bucket = &self.index.buckets[number];
        let path = self.dir.join(&bucket.file);
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        decode_bucket(&bytes, bucket.episodes).with_context(|| format!("decoding {}", path.display()))
    }

    /// Hands every bucket to `work`, in order. The next bucket is read and decoded on another
    /// thread while `work` runs, so at most two buckets are in memory.
    pub fn for_each_bucket(&self, mut work: impl FnMut(&[QuestEpisode]) -> Result<()>) -> Result<()> {
        let (loaded, received) = mpsc::sync_channel(1);
        std::thread::scope(|scope| {
            scope.spawn(move || {
                for number in 0..self.bucket_count() {
                    // Fails once `work` failed and stopped listening: nothing left to do.
                    if loaded.send(self.load_bucket(number)).is_err() {
                        return;
                    }
                }
            });
            for bucket in received {
                work(&bucket?)?;
            }
            Ok(())
        })
    }

    /// Every episode at once (tests).
    #[cfg(test)]
    pub fn load_all(&self) -> Result<Vec<QuestEpisode>> {
        let mut episodes = Vec::with_capacity(self.episode_count());
        for number in 0..self.bucket_count() {
            episodes.extend(self.load_bucket(number)?);
        }
        Ok(episodes)
    }
}

fn decode_bucket(bytes: &[u8], expected: usize) -> Result<Vec<QuestEpisode>> {
    let mut input = Input::new(bytes);
    let mut episodes = Vec::with_capacity(expected);
    while !input.is_empty() {
        let number = episodes.len();
        let length = usize::try_from(read_varint(&mut input)?).context("episode length out of range")?;
        let mut episode_bytes = Input::new(input.take(length).with_context(|| format!("episode {number}"))?);
        let episode = QuestEpisode::decode(&mut episode_bytes).with_context(|| format!("episode {number}"))?;
        if !episode_bytes.is_empty() {
            bail!("episode {number} has bytes left over");
        }
        episodes.push(episode);
    }
    if episodes.len() != expected {
        bail!("{} episodes, the index says {expected}", episodes.len());
    }
    Ok(episodes)
}

/// Writes a store from episodes pushed in ingest's order. The store appears at `dir` only once
/// [`finish`](StoreWriter::finish) succeeds; until then it is written next to it.
pub struct StoreWriter {
    dir: PathBuf,
    staging: PathBuf,
    bucket_bytes: usize,
    index: Index,
    bucket: Vec<u8>,
    bucket_episodes: usize,
    bucket_characters: usize,
    /// The character of the last episode pushed: (contributor id, character key).
    character: Option<(String, String)>,
}

impl StoreWriter {
    /// A writer whose buckets end at the first character boundary after `bucket_bytes`.
    pub fn create(dir: &Path, bucket_bytes: usize) -> Result<StoreWriter> {
        let staging = sibling(dir, "staging");
        if staging.exists() {
            fs::remove_dir_all(&staging).with_context(|| format!("removing {}", staging.display()))?;
        }
        fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;
        Ok(StoreWriter {
            dir: dir.to_owned(),
            staging,
            bucket_bytes,
            index: Index {
                format: FORMAT_VERSION,
                ..Index::default()
            },
            bucket: Vec::new(),
            bucket_episodes: 0,
            bucket_characters: 0,
            character: None,
        })
    }

    /// Adds the next episode. Characters must come one after another, in ascending (contributor
    /// id, character key) order, as ingest sorts them: a character met again later would be split.
    pub fn push(&mut self, episode: &QuestEpisode) -> Result<()> {
        let same_character = self
            .character
            .as_ref()
            .is_some_and(|(contributor, character)| *contributor == episode.contributor_id && *character == episode.character_key);
        if !same_character {
            if let Some(previous) = &self.character {
                if (previous.0.as_str(), previous.1.as_str()) >= (episode.contributor_id.as_str(), episode.character_key.as_str()) {
                    bail!(
                        "episodes out of order: character {}/{} after {}/{}",
                        episode.contributor_id,
                        episode.character_key,
                        previous.0,
                        previous.1
                    );
                }
            }
            if self.bucket.len() >= self.bucket_bytes {
                self.close_bucket()?;
            }
            self.character = Some((episode.contributor_id.clone(), episode.character_key.clone()));
            self.bucket_characters += 1;
            self.index.characters += 1;
        }
        let mut encoded = Vec::new();
        episode.encode(&mut encoded);
        write_varint(&mut self.bucket, encoded.len() as u64);
        self.bucket.extend_from_slice(&encoded);
        self.bucket_episodes += 1;
        self.index.episodes += 1;
        Ok(())
    }

    fn close_bucket(&mut self) -> Result<()> {
        if self.bucket_episodes == 0 {
            return Ok(());
        }
        let file = format!("{:05}.bin", self.index.buckets.len());
        let path = self.staging.join(&file);
        fs::write(&path, &self.bucket).with_context(|| format!("writing {}", path.display()))?;
        self.index.buckets.push(Bucket {
            file,
            episodes: self.bucket_episodes,
            characters: self.bucket_characters,
            bytes: self.bucket.len(),
        });
        self.bucket.clear();
        self.bucket_episodes = 0;
        self.bucket_characters = 0;
        Ok(())
    }

    /// Writes the last bucket and the index, and puts the store in place of any earlier one.
    pub fn finish(mut self) -> Result<EpisodeStore> {
        self.close_bucket()?;
        let index_path = self.staging.join("index.json");
        fs::write(&index_path, serde_json::to_vec_pretty(&self.index)?).with_context(|| format!("writing {}", index_path.display()))?;
        let previous = sibling(&self.dir, "previous");
        if previous.exists() {
            fs::remove_dir_all(&previous).with_context(|| format!("removing {}", previous.display()))?;
        }
        if self.dir.exists() {
            fs::rename(&self.dir, &previous).with_context(|| format!("moving {} aside", self.dir.display()))?;
        }
        fs::rename(&self.staging, &self.dir).with_context(|| format!("moving the new store to {}", self.dir.display()))?;
        if previous.exists() {
            fs::remove_dir_all(&previous).with_context(|| format!("removing {}", previous.display()))?;
        }
        Ok(EpisodeStore {
            dir: self.dir,
            index: self.index,
        })
    }
}

/// `<dir>.<suffix>`, next to `dir`.
fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    dir.with_file_name(name)
}

/// Writes a store of `episodes`, which are in ingest's order.
pub fn write(dir: &Path, episodes: &[QuestEpisode], bucket_bytes: usize) -> Result<EpisodeStore> {
    let mut writer = StoreWriter::create(dir, bucket_bytes)?;
    for episode in episodes {
        writer.push(episode)?;
    }
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::make_episode;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A directory of its own under the system's temporary directory, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Scratch {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!("episode-store-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Episode `key` of character `character` of contributor `contributor`.
    fn episode(contributor: &str, character: &str, key: &str) -> QuestEpisode {
        QuestEpisode {
            key: key.into(),
            contributor_id: contributor.into(),
            character_key: character.into(),
            ..make_episode()
        }
    }

    fn keys(episodes: &[QuestEpisode]) -> Vec<&str> {
        episodes.iter().map(|episode| episode.key.as_str()).collect()
    }

    fn sample() -> Vec<QuestEpisode> {
        vec![
            episode("a", "a:1", "e1"),
            episode("a", "a:1", "e2"),
            episode("a", "a:1", "e3"),
            episode("a", "a:2", "e4"),
            episode("b", "b:1", "e5"),
            episode("b", "b:1", "e6"),
        ]
    }

    fn write_small(dir: &Path, episodes: &[QuestEpisode]) -> Result<EpisodeStore> {
        // One byte: every character boundary ends a bucket.
        let mut writer = StoreWriter::create(dir, 1)?;
        for episode in episodes {
            writer.push(episode)?;
        }
        writer.finish()
    }

    #[test]
    fn reads_back_every_episode_in_order_and_never_splits_a_character() {
        let scratch = Scratch::new();
        let episodes = sample();
        let store = write_small(&scratch.0.join("episodes"), &episodes).unwrap();
        assert_eq!((store.episode_count(), store.character_count(), store.bucket_count()), (6, 3, 3));

        let reopened = EpisodeStore::open(&scratch.0.join("episodes")).unwrap();
        let mut buckets = Vec::new();
        reopened
            .for_each_bucket(|bucket| {
                buckets.push(keys(bucket).join(","));
                Ok(())
            })
            .unwrap();
        assert_eq!(buckets, ["e1,e2,e3", "e4", "e5,e6"]);
        assert_eq!(reopened.load_all().unwrap(), episodes, "every field reads back");
    }

    #[test]
    fn a_large_bucket_holds_several_characters() {
        let scratch = Scratch::new();
        let store = write(&scratch.0.join("episodes"), &sample(), BUCKET_BYTES).unwrap();
        assert_eq!(store.bucket_count(), 1);
        assert_eq!(keys(&store.load_bucket(0).unwrap()), ["e1", "e2", "e3", "e4", "e5", "e6"]);
    }

    #[test]
    fn refuses_characters_out_of_order() {
        let scratch = Scratch::new();
        let mut episodes = sample();
        // Character a:1 again after a:2 would be split over two places.
        episodes.push(episode("a", "a:1", "e7"));
        let error = write_small(&scratch.0.join("episodes"), &episodes).err().expect("out of order");
        assert!(format!("{error:#}").contains("out of order"), "{error:#}");
        assert!(!scratch.0.join("episodes").exists(), "nothing half-written in place");
    }

    #[test]
    fn replaces_an_earlier_store_and_leaves_no_staging_behind() {
        let scratch = Scratch::new();
        let dir = scratch.0.join("episodes");
        write_small(&dir, &sample()).unwrap();
        write_small(&dir, &sample()[4..]).unwrap();
        assert_eq!(keys(&EpisodeStore::open(&dir).unwrap().load_all().unwrap()), ["e5", "e6"]);
        let left: Vec<_> = fs::read_dir(&scratch.0).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(left, ["episodes"]);
        assert!(!dir.join("00001.bin").exists(), "the earlier store's buckets are gone");
    }

    #[test]
    fn a_damaged_bucket_or_another_format_fails_with_the_file_named() {
        let scratch = Scratch::new();
        let dir = scratch.0.join("episodes");
        write_small(&dir, &sample()).unwrap();
        let bucket = dir.join("00000.bin");
        let bytes = fs::read(&bucket).unwrap();
        fs::write(&bucket, &bytes[..bytes.len() - 3]).unwrap();
        let store = EpisodeStore::open(&dir).unwrap();
        let error = format!("{:#}", store.for_each_bucket(|_| Ok(())).unwrap_err());
        assert!(error.contains("00000.bin"), "{error}");

        let index = dir.join("index.json");
        let text = fs::read_to_string(&index).unwrap().replace("\"format\": 1", "\"format\": 0");
        fs::write(&index, text).unwrap();
        let error = format!("{:#}", EpisodeStore::open(&dir).err().unwrap());
        assert!(error.contains("format 0"), "{error}");
    }

    #[test]
    fn stops_reading_when_the_work_fails() {
        let scratch = Scratch::new();
        let store = write_small(&scratch.0.join("episodes"), &sample()).unwrap();
        let mut seen = 0;
        let error = store
            .for_each_bucket(|_| {
                seen += 1;
                bail!("the work failed")
            })
            .unwrap_err();
        assert_eq!(seen, 1);
        assert_eq!(error.to_string(), "the work failed");
    }
}
