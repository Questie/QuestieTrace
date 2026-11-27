//! Everything combine reads: every signal's candidates file, the catalog for names and givers, and
//! the ground truth for calibration and metrics.

use super::edges::{truth_lookup, QuestDomains, Truth};
use super::features::Coverage;
use crate::model::{CandidateFile, GroundTruth, QuestCatalog, QuestId};
use crate::signals::registry;
use crate::text::js_trim;
use anyhow::{bail, Result};

pub struct Inputs<'a> {
    /// One per signal, in signal-name order.
    pub files: Vec<&'a CandidateFile>,
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

pub fn prepare_inputs<'a>(all_files: &'a [CandidateFile], catalog: &'a QuestCatalog, ground_truth: &'a GroundTruth) -> Result<Inputs<'a>> {
    if all_files.is_empty() {
        bail!("no candidate files; run the signals first");
    }
    // Every file counts as evidence, so only the signals' files may be here. Anything else is left
    // from an older run, or not recorded in game at all. Wowhead's claims must never come back this
    // way: Questie's own relations were scraped from its Series rows, so they agree with the ground
    // truth whether they are right or not.
    let signals = registry();
    if let Some(file) = all_files.iter().find(|file| signals.iter().all(|signal| signal.name() != file.signal)) {
        bail!("the candidate file of {:?} is not from a signal; remove it from candidates/", file.signal);
    }
    // Signal-name order decides edge order, and with it every tie in the model and resolution.
    // This is the one place that orders the files; callers may pass them in any order, as names
    // are unique.
    let mut files: Vec<&CandidateFile> = all_files.iter().collect();
    files.sort_by(|a, b| a.signal.cmp(&b.signal));
    if let Some(pair) = files.windows(2).find(|pair| pair[0].signal == pair[1].signal) {
        bail!("two candidate files are both signal {:?}; remove the stale one", pair[0].signal);
    }
    Ok(Inputs {
        coverage: files
            .iter()
            .map(|file| (file.signal.as_str(), file.covered_quest_ids.iter().copied().collect()))
            .collect(),
        files,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_file(signal: &str) -> CandidateFile {
        CandidateFile {
            signal: signal.into(),
            generated_at: String::new(),
            input_count: 0,
            params: serde_json::Value::Null,
            covered_quest_ids: vec![],
            candidates: vec![],
        }
    }

    #[test]
    fn refuses_two_files_of_one_signal() {
        // A copy left next to the real file (candidates/handoff-old.json still says "handoff").
        let files = [empty_file("offer-set"), empty_file("handoff"), empty_file("handoff")];
        let error = prepare_inputs(&files, &QuestCatalog::default(), &GroundTruth::default())
            .err()
            .expect("a duplicate signal");
        assert!(error.to_string().contains("\"handoff\""), "{error}");
    }

    /// `candidates/` of a run made before November 25, 2026 holds Wowhead's claims. As evidence they
    /// would decide relations.
    #[test]
    fn refuses_candidates_that_no_signal_made() {
        let files = [empty_file("handoff"), empty_file("wowhead-series")];
        let error = prepare_inputs(&files, &QuestCatalog::default(), &GroundTruth::default())
            .err()
            .expect("a file no signal made");
        assert!(error.to_string().contains("\"wowhead-series\""), "{error}");
    }

    #[test]
    fn every_registered_signal_has_its_own_name_and_is_taken_as_evidence() {
        let names: Vec<&str> = registry().iter().map(|signal| signal.name()).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "{names:?}");
        let files: Vec<CandidateFile> = names.iter().map(|name| empty_file(name)).collect();
        let (catalog, truth) = (QuestCatalog::default(), GroundTruth::default());
        let inputs = prepare_inputs(&files, &catalog, &truth).unwrap();
        assert_eq!(inputs.files.iter().map(|file| file.signal.as_str()).collect::<Vec<_>>(), unique);
    }
}
