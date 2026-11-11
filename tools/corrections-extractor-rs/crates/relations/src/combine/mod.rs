//! Combine: every signal's candidates -> one resolved set of relations, and the deliverables:
//! `foreverQuestRelationTraces.lua`, `relations-review.md` and `relations-meta.json`.

use crate::config::Config;
use crate::model::{CandidateFile, GroundTruth, QuestCatalog, QuestEpisode};
use anyhow::{bail, Result};

pub struct CombineInput<'a> {
    /// Every candidates file (signals and sources), sorted by signal name.
    pub candidate_files: Vec<CandidateFile>,
    pub catalog: &'a QuestCatalog,
    pub ground_truth: &'a GroundTruth,
    /// For counts only.
    pub episodes: &'a [QuestEpisode],
}

pub struct CombineOutput {}

impl CombineOutput {
    pub fn write(&self, _config: &Config) -> Result<()> {
        bail!("combine is not ported yet")
    }
}

pub fn run(_config: &Config, _input: CombineInput) -> Result<CombineOutput> {
    bail!("combine is not ported yet")
}
