//! Wowhead relations from the scraper's databases, one candidates file per page surface
//! (`wowhead-series`, `wowhead-storyline`, ...). Context only: see combine.

use crate::config::Config;
use crate::io;
use crate::model::CandidateFile;
use anyhow::{bail, Result};

pub struct WowheadOutput {
    pub files: Vec<CandidateFile>,
    /// reports/wowhead-inventory.json
    pub inventory: serde_json::Value,
}

impl WowheadOutput {
    pub fn write(&self, config: &Config) -> Result<()> {
        for file in &self.files {
            io::write_json(&config.candidates(&file.signal), file)?;
        }
        io::write_json(&config.report("wowhead-inventory"), &self.inventory)
    }
}

pub fn run(config: &Config) -> Result<WowheadOutput> {
    let _ = config.scraper_dir()?;
    bail!("the wowhead source is not ported yet")
}
