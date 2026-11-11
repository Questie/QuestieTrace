//! Reading and writing the pipeline's files. Every write goes through a temporary file and a
//! rename, so a reader never sees a half-written file.

use crate::model::{CandidateFile, QuestEpisode};
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

fn temporary(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Writes `write`'s output to `path` atomically, creating the directory.
pub fn write_atomic(path: &Path, write: impl FnOnce(&mut BufWriter<File>) -> Result<()>) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = temporary(path);
    let result = (|| -> Result<()> {
        let mut out = BufWriter::new(File::create(&tmp)?);
        write(&mut out)?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        // Best effort: the write error is the one worth reporting.
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {}", path.display()))
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    write_atomic(path, |out| Ok(serde_json::to_writer(out, value)?))
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    write_atomic(path, |out| Ok(out.write_all(text.as_bytes())?))
}

/// Reads a JSON file another step produced; `produced_by` names that step in the error.
pub fn read_json<T: DeserializeOwned>(path: &Path, produced_by: &str) -> Result<T> {
    let file = File::open(path).with_context(|| format!("{} is missing: run `relations {produced_by}` first", path.display()))?;
    serde_json::from_reader(BufReader::new(file)).with_context(|| format!("reading {}", path.display()))
}

/// One episode per line.
pub fn write_episodes(path: &Path, episodes: &[QuestEpisode]) -> Result<()> {
    write_atomic(path, |out| {
        for episode in episodes {
            serde_json::to_writer(&mut *out, episode)?;
            out.write_all(b"\n")?;
        }
        Ok(())
    })
}

pub fn read_episodes(path: &Path) -> Result<Vec<QuestEpisode>> {
    let file = File::open(path).with_context(|| format!("{} is missing: run `relations ingest` first", path.display()))?;
    let mut episodes = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.with_context(|| format!("reading {}", path.display()))?;
        if !line.is_empty() {
            episodes.push(serde_json::from_str(&line).with_context(|| format!("{} line {}", path.display(), index + 1))?);
        }
    }
    Ok(episodes)
}

/// Every candidates/*.json file, sorted by signal name.
pub fn read_candidate_files(dir: &Path) -> Result<Vec<CandidateFile>> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("{} is missing: run the signals first", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    // By signal name (the file stem), not by path: "wowhead-requires" sorts before
    // "wowhead-requires-any", as the TS pipeline orders them.
    paths.sort_by(|a, b| a.file_stem().cmp(&b.file_stem()));
    paths.iter().map(|path| read_json(path, "pipeline")).collect()
}
