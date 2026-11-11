//! Catalog: QuestieDB (through its own Lua loader) -> the quests Forever knows, with the facts
//! signals gate on, plus the ground truth that scoring and combine compare against.

use crate::config::Config;
use crate::io;
use crate::model::{GroundTruth, QuestCatalog};
use anyhow::{bail, Result};

pub struct CatalogOutput {
    pub catalog: QuestCatalog,
    pub ground_truth: GroundTruth,
    /// reports/catalog.json
    pub summary: serde_json::Value,
}

impl CatalogOutput {
    pub fn write(&self, config: &Config) -> Result<()> {
        io::write_json(&config.catalog(), &self.catalog)?;
        io::write_json(&config.ground_truth(), &self.ground_truth)?;
        io::write_json(&config.report("catalog"), &self.summary)
    }
}

pub fn run(config: &Config) -> Result<CatalogOutput> {
    let _ = config.questie_db_dir()?;
    bail!("catalog is not ported yet")
}
