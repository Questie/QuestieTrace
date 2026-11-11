//! Everything combine reads: every candidates file (whatever signals exist), the catalog for names
//! and givers, and the ground truth for calibration and metrics.
//!
//! Wowhead files are split off here into a context-only channel. Questie's own relation data was
//! scraped from Wowhead Series rows, so Wowhead agreeing with the ground truth is circular and its
//! measured precision means nothing. Its claims are shown to reviewers next to the trace evidence,
//! and nothing else: no model column, no coverage or silence, no say in any resolution rule.

use super::edges::{collect_edges, truth_lookup, Edge, EdgeKey, QuestDomains, Truth};
use super::features::Coverage;
use crate::model::{CandidateFile, GroundTruth, QuestCatalog, QuestId};
use crate::text::js_trim;
use anyhow::{bail, Result};
use indexmap::IndexMap;

pub struct Inputs<'a> {
    /// Candidate files the model and resolution use.
    pub files: Vec<&'a CandidateFile>,
    /// Context-only files (Wowhead), never used for scoring or decisions.
    pub context_files: Vec<&'a CandidateFile>,
    /// Canonical edges claimed by the context-only files, keyed like the model's edges.
    pub context: IndexMap<EdgeKey, Edge<'a>>,
    pub catalog: &'a QuestCatalog,
    pub truth: Truth<'a>,
    pub domains: QuestDomains,
    pub coverage: Coverage<'a>,
}

impl Inputs<'_> {
    pub fn family_of(&self, quest_id: QuestId) -> String {
        family_of(self.catalog, quest_id)
    }
}

/// Wowhead's files, by name: every source added to that channel must match here, no signal may.
pub fn is_context_only(signal: &str) -> bool {
    signal == "wowhead" || signal.starts_with("wowhead-")
}

pub fn prepare_inputs<'a>(all_files: &'a [CandidateFile], catalog: &'a QuestCatalog, ground_truth: &'a GroundTruth) -> Result<Inputs<'a>> {
    if all_files.is_empty() {
        bail!("no candidate files; run the signals first");
    }
    // Signal-name order decides edge order, and with it every tie in the model and resolution.
    // This is the one place that orders the files; callers may pass them in any order. Sorting
    // file paths is not the same: "wowhead-requires-any.json" < "wowhead-requires.json".
    let mut sorted: Vec<&CandidateFile> = all_files.iter().collect();
    sorted.sort_by(|a, b| a.signal.cmp(&b.signal));
    let (context_files, files): (Vec<&CandidateFile>, Vec<&CandidateFile>) = sorted.into_iter().partition(|file| is_context_only(&file.signal));
    if files.is_empty() {
        bail!("no trace signal candidate files; Wowhead alone is context only");
    }
    Ok(Inputs {
        context: collect_edges(context_files.iter().copied()),
        coverage: files
            .iter()
            .map(|file| (file.signal.as_str(), file.covered_quest_ids.iter().copied().collect()))
            .collect(),
        files,
        context_files,
        catalog,
        truth: truth_lookup(ground_truth),
        domains: QuestDomains::new(ground_truth),
    })
}

/// Same-name copies share a family (faction/race/profession variants of one quest).
pub fn family_of(catalog: &QuestCatalog, quest_id: QuestId) -> String {
    match catalog.quest(quest_id).and_then(|quest| quest.name.as_deref()).filter(|name| !name.is_empty()) {
        Some(name) => format!("name:{}", js_trim(name).to_lowercase()),
        None => format!("id:{quest_id}"),
    }
}
