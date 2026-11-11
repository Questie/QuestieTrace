//! Signals: each reads every episode and the catalog and claims relationship edges with its own
//! evidence. They never read the ground truth (that is for scoring and combine only), and they
//! are independent of each other, so the pipeline runs them side by side.
//!
//! Adding a signal: write a module with a type implementing [`Signal`], and list it in
//! [`registry`]. The pipeline runs it, writes `candidates/<name>.json` and its reports, and
//! combine picks its candidates up like any other signal's.

pub mod cooccurrence;

use crate::config::Config;
use crate::io;
use crate::model::{CandidateFile, QuestCatalog, QuestEpisode};
use anyhow::Result;

pub mod handoff;
pub mod offer_set;
pub mod breadcrumbs;

/// What every signal reads.
pub struct SignalInput<'a> {
    pub episodes: &'a [QuestEpisode],
    pub catalog: &'a QuestCatalog,
    /// The run's start time, for `generatedAt`.
    pub generated_at: &'a str,
}

/// A signal's result: its candidates, and reports for people tuning it.
pub struct SignalOutput {
    pub candidates: CandidateFile,
    /// Written to `reports/<name>.json`, e.g. ("handoff-timing", {...}).
    pub reports: Vec<(&'static str, serde_json::Value)>,
}

impl SignalOutput {
    pub fn write(&self, config: &Config) -> Result<()> {
        io::write_json(&config.candidates(&self.candidates.signal), &self.candidates)?;
        for (name, report) in &self.reports {
            io::write_json(&config.report(name), report)?;
        }
        Ok(())
    }
}

pub trait Signal: Sync {
    /// The candidates file name, `candidates/<name>.json`, and the `signal:<name>` command.
    fn name(&self) -> &'static str;
    /// One line for the command list.
    fn describe(&self) -> &'static str;
    fn run(&self, input: &SignalInput) -> SignalOutput;
}

/// Every signal, in the order the pipeline lists them.
pub fn registry() -> Vec<&'static dyn Signal> {
    vec![&handoff::Handoff, &offer_set::OfferSet, &cooccurrence::Cooccurrence, &breadcrumbs::Breadcrumbs]
}

/// Runs every signal on all cores; results in registry order.
pub fn run_all(input: &SignalInput) -> Vec<SignalOutput> {
    use rayon::prelude::*;
    registry().par_iter().map(|signal| signal.run(input)).collect()
}
