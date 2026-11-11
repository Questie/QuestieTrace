//! Catalog: QuestieDB (through its own Lua loader) -> the quests Forever knows, with the facts
//! signals gate on, plus the ground truth that scoring and combine compare against.
//!
//! `materialize` runs QuestieDB's loader and correction registry (`forever-quests.lua`), so the
//! layering is QuestieDB's own; `build` shapes the rows into the [`QuestCatalog`] and
//! [`GroundTruth`] contracts; `summary` counts what came out (reports/catalog.json).

pub mod build;
pub mod materialize;
pub mod summary;

use crate::config::Config;
use crate::io;
use crate::model::{GroundTruth, QuestCatalog};
use anyhow::Result;
use materialize::{materialize_forever_quests, MaterializeOptions};
use summary::CatalogSummary;

pub struct CatalogOutput {
    pub catalog: QuestCatalog,
    pub ground_truth: GroundTruth,
    /// reports/catalog.json
    pub summary: CatalogSummary,
}

impl CatalogOutput {
    pub fn write(&self, config: &Config) -> Result<()> {
        io::write_json(&config.catalog(), &self.catalog)?;
        io::write_json(&config.ground_truth(), &self.ground_truth)?;
        io::write_json(&config.report("catalog"), &self.summary)?;
        println!("{}", self.summary);
        println!("wrote {}, {}", config.catalog().display(), config.ground_truth().display());
        Ok(())
    }
}

pub fn run(config: &Config) -> Result<CatalogOutput> {
    let options = MaterializeOptions {
        trace_modules_dir: config.trace_modules_dir.as_deref(),
        fixture: None,
    };
    let materialized = materialize_forever_quests(config.questie_db_dir()?, &build::exported_fields(), options)?;
    let catalog = build::build_catalog(&materialized, &config.generated_at);
    let ground_truth = build::build_ground_truth(&materialized, &config.generated_at);
    let summary = summary::summarize(&materialized, &catalog, &ground_truth);
    Ok(CatalogOutput {
        catalog,
        ground_truth,
        summary,
    })
}
