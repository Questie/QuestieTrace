//! Signals: each reads every episode and the catalog and claims relationship edges with its own
//! evidence. They never read the ground truth (that is for scoring and combine only), and they
//! are independent of each other, so the pipeline runs them side by side.
//!
//! Adding a signal: write a module with a type implementing [`Signal`], and list it in
//! [`registry`]. The pipeline runs it, writes `candidates/<name>.json` and its reports, and
//! combine picks its candidates up like any other signal's. A signal returns only what it found
//! ([`Findings`]); [`run_one`] adds what every candidates file shares (its name, the run's time).

use crate::config::Config;
use crate::io;
use crate::model::{CandidateFile, QuestCatalog, QuestEpisode, QuestId, RelationCandidate};
use anyhow::Result;

pub mod absence;
pub mod breadcrumbs;
pub mod cooccurrence;
pub mod handoff;
pub mod offer_set;

/// What every signal reads.
pub struct SignalInput<'a> {
    pub episodes: &'a [QuestEpisode],
    pub catalog: &'a QuestCatalog,
    /// The run's start time, for `generatedAt`.
    pub generated_at: &'a str,
}

/// What a signal found: its own part of `candidates/<name>.json`, and reports for people tuning it.
pub struct Findings {
    /// Episodes the signal looked at.
    pub input_count: usize,
    /// The tunables the signal ran with, for reproducibility: its params struct (plus any notes on
    /// what the counts mean) through [`to_json`], so a new tunable cannot go missing from the file.
    pub params: serde_json::Value,
    /// Every quest the signal could judge, with or without a candidate: recall depends on it.
    pub covered_quest_ids: Vec<QuestId>,
    pub candidates: Vec<RelationCandidate>,
    /// Written to `reports/<name>.json`, e.g. ("handoff-timing", {...}).
    pub reports: Vec<(&'static str, serde_json::Value)>,
}

/// A signal's files: its candidates file and its reports.
pub struct SignalOutput {
    pub candidates: CandidateFile,
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
    /// What the signal found; [`run_one`] makes its files of it.
    fn run(&self, input: &SignalInput) -> Findings;
}

/// Every signal, in the order the pipeline lists them.
pub fn registry() -> Vec<&'static dyn Signal> {
    vec![
        &handoff::Handoff,
        &offer_set::OfferSet,
        &absence::Absence,
        &cooccurrence::Cooccurrence,
        &breadcrumbs::Breadcrumbs,
    ]
}

/// Params and reports as JSON. They are plain data (numbers, strings, lists, maps keyed by strings
/// or integers), which always serializes.
pub(crate) fn to_json(value: &impl serde::Serialize) -> serde_json::Value {
    serde_json::to_value(value).expect("signal params and reports are JSON-representable")
}

/// Runs one signal and fills in what every candidates file shares: its name and the run's time.
pub fn run_one(signal: &dyn Signal, input: &SignalInput) -> SignalOutput {
    let Findings {
        input_count,
        params,
        covered_quest_ids,
        candidates,
        reports,
    } = signal.run(input);
    SignalOutput {
        candidates: CandidateFile {
            signal: signal.name().to_owned(),
            generated_at: input.generated_at.to_owned(),
            input_count,
            params,
            covered_quest_ids,
            candidates,
        },
        reports,
    }
}

/// Runs every signal, one after another (each uses every core itself), and hands each result to
/// `write` as soon as it is done. Side by side, their working sets added up: on the full archive
/// that is more memory than the episodes themselves several times over.
pub fn run_all(input: &SignalInput, mut write: impl FnMut(SignalOutput) -> Result<()>) -> Result<()> {
    for signal in registry() {
        let started = std::time::Instant::now();
        write(run_one(signal, input))?;
        eprintln!("== signal:{} done in {:.1}s", signal.name(), started.elapsed().as_secs_f64());
    }
    Ok(())
}
