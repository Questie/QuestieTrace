//! `signal:offer-set`: completed sets at offer time -> prerequisites.
//!
//! The server only offers a quest when every prerequisite is completed, so what a character had
//! completed whenever it could take quest X contains X's prerequisites. The steps:
//!
//! - [`observe`]: episodes -> one view per quest and character (its sightings of the quest).
//! - [`aggregate`]: views -> per-candidate counts for each quest.
//! - [`infer`]: counts -> prerequisite edges.
//! - [`score`]: edge features -> calibrated score.
//!
//! The TypeScript command's `--dump` (every edge with its features, for calibration) and
//! `--param` overrides are not ported: the pipeline runs every signal with [`PARAMS`].

mod aggregate;
mod infer;
mod observe;
mod score;
mod variants;

use super::{Signal, SignalInput, SignalOutput};
use crate::model::{CandidateFile, QuestId, RelationCandidate};
use aggregate::{aggregate_quest, FinisherIndex, QuestAssessment};
use infer::{infer_quest, to_candidate};
use observe::{group_by_character, observe_character, prerequisite_candidates, CharacterView, ObserveStats};
use rayon::prelude::*;
use score::score_edge;
use serde_json::json;
use std::collections::BTreeMap;
use variants::VariantIndex;

/// The signal's tunables.
pub struct Params {
    /// Seconds after a sighting that a completion still counts. A turn-in counts at its event
    /// time, but co-completed copies only arrive with the completed-quest delta 0.1-0.4s later,
    /// sometimes after the next offer was recorded.
    pub completion_tolerance_seconds: f64,
    /// A completion this many seconds or less before the first sighting counts as "fresh".
    pub fresh_seconds: f64,
    /// ...and this many seconds or less as "immediate" (turned in, next quest offered right away).
    pub immediate_seconds: f64,
    /// Completions this close to the most recent one share "latest" (variants completed together).
    pub latest_tie_seconds: f64,
    /// Characters allowed to lack P at a sighting before P is dropped (absorbs bad histories).
    pub max_contradictions: u32,
    /// Quests seen by fewer independent characters are not judged (and not covered).
    pub min_characters: u32,
    /// Minimum share of window characters that completed P after reaching X's giver.
    pub min_pending_rate: f64,
    /// Secondary edges need at least this many supporting characters.
    pub min_secondary_support: u32,
}

pub const PARAMS: Params = Params {
    completion_tolerance_seconds: 1.0,
    fresh_seconds: 900.0,
    immediate_seconds: 30.0,
    latest_tie_seconds: 2.0,
    max_contradictions: 0,
    min_characters: 2,
    min_pending_rate: 0.5,
    min_secondary_support: 5,
};

pub struct OfferSet;

impl Signal for OfferSet {
    fn name(&self) -> &'static str {
        "offer-set"
    }

    fn describe(&self) -> &'static str {
        "completed sets at offer time -> prerequisites"
    }

    fn run(&self, input: &SignalInput) -> SignalOutput {
        let params = &PARAMS;
        let catalog = input.catalog;
        let mut stats = ObserveStats::default();
        let characters: Vec<_> = group_by_character(input.episodes, &mut stats).into_values().collect();

        let candidates = prerequisite_candidates(catalog);
        let views: Vec<Vec<CharacterView>> = characters
            .par_iter()
            .map(|episodes| observe_character(episodes, catalog, &candidates, params.completion_tolerance_seconds))
            .collect();
        // Per quest, the views in character order (evidence lists the first characters).
        let mut views_by_quest: BTreeMap<QuestId, Vec<CharacterView>> = BTreeMap::new();
        for view in views.into_iter().flatten() {
            stats.views += 1;
            views_by_quest.entry(view.quest_id).or_default().push(view);
        }

        let variants = VariantIndex::new(catalog);
        let finishers = FinisherIndex::new(catalog, characters.iter().flatten().copied());
        let assessments: BTreeMap<QuestId, QuestAssessment> = views_by_quest
            .par_iter()
            .map(|(&quest_id, views)| (quest_id, aggregate_quest(quest_id, views, catalog, params, &finishers, &variants)))
            .collect();

        let mut covered = Vec::new();
        let mut candidates: Vec<RelationCandidate> = Vec::new();
        for assessment in assessments.values().filter(|assessment| assessment.characters >= params.min_characters) {
            covered.push(assessment.quest_id);
            let edges = infer_quest(assessment, &assessments, &variants, params);
            candidates.extend(edges.iter().map(|edge| to_candidate(edge, score_edge(&edge.features))));
        }
        candidates.sort_by(|a, b| a.quest_id.cmp(&b.quest_id).then(b.score.total_cmp(&a.score)).then(a.target.cmp(&b.target)));

        eprintln!(
            "offer-set: {} characters, {} quests seen, {} covered, {} candidates",
            stats.characters,
            views_by_quest.len(),
            covered.len(),
            candidates.len()
        );
        let summary = json!({
            "generatedAt": input.generated_at,
            "observe": stats,
            "questsSeen": views_by_quest.len(),
            "covered": covered.len(),
            "candidates": candidates.len(),
        });
        SignalOutput {
            candidates: CandidateFile {
                signal: self.name().to_string(),
                generated_at: input.generated_at.to_string(),
                input_count: stats.usable_episodes,
                params: json!({
                    "completionToleranceSeconds": params.completion_tolerance_seconds,
                    "freshSeconds": params.fresh_seconds,
                    "immediateSeconds": params.immediate_seconds,
                    "latestTieSeconds": params.latest_tie_seconds,
                    "maxContradictions": params.max_contradictions,
                    "minCharacters": params.min_characters,
                    "minPendingRate": params.min_pending_rate,
                    "minSecondarySupport": params.min_secondary_support,
                    "support": "independent characters",
                    // The TS signal's value, kept so both files say the same.
                    "score": "calibrated lookup, see signals/offer-set/score.ts",
                }),
                covered_quest_ids: covered,
                candidates,
            },
            reports: vec![("offer-set-summary", summary)],
        }
    }
}
