//! Ingest: trace-data submissions -> one [`QuestEpisode`] per recording session, its copies across
//! submissions merged, its character worked out.

use crate::config::Config;
use crate::io;
use crate::model::QuestEpisode;
use anyhow::{bail, Result};

pub struct IngestOutput {
    pub episodes: Vec<QuestEpisode>,
    /// reports/ingest.json
    pub report: serde_json::Value,
}

impl IngestOutput {
    pub fn write(&self, config: &Config) -> Result<()> {
        io::write_episodes(&config.episodes(), &self.episodes)?;
        io::write_json(&config.report("ingest"), &self.report)
    }
}

pub fn run(config: &Config) -> Result<IngestOutput> {
    let _ = config.trace_data_dir()?;
    bail!("ingest is not ported yet")
}
