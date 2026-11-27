//! `signal:offer-set`: completed sets at offer time -> prerequisites.
//!
//! The server only offers a quest when every prerequisite is completed, so what a character had
//! completed whenever it could take quest X contains X's prerequisites. The steps:
//!
//! - [`observe`]: episodes -> one view per quest and character (its sightings of the quest).
//! - [`aggregate`]: views, folded per quest in character order -> per-candidate counts.
//! - [`infer`]: counts -> prerequisite edges.
//! - [`score`]: edge features -> calibrated score.
//!
//! Characters are observed a chunk at a time and their views folded right away, so only one
//! chunk's views exist at once: all views together are several times the episodes' size.
//!
//! The TypeScript command's `--dump` (every edge with its features, for calibration) and
//! `--param` overrides are not ported: the pipeline runs every signal with [`PARAMS`].

mod aggregate;
mod infer;
mod observe;
mod score;
mod variants;

use super::{to_json, Findings, Signal, SignalInput};
use crate::model::{QuestId, RelationCandidate};
use aggregate::{FinisherIndex, QuestAssessment, QuestFold};
use anyhow::Result;
use infer::{infer_quest, to_candidate};
use observe::{group_by_character, observe_character, prerequisite_candidates, CharacterView, ObserveStats};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use score::score_edge;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use variants::VariantIndex;

/// Characters observed per chunk: enough to keep every core busy, few enough that their views
/// stay small next to the episodes.
const CHARACTERS_PER_CHUNK: usize = 512;

/// The signal's tunables.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
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

/// candidates/offer-set.json `params`: the tunables and what the numbers mean.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileParams<'a> {
    #[serde(flatten)]
    tunables: &'a Params,
    support: &'static str,
    score: &'static str,
}

pub struct OfferSet;

impl Signal for OfferSet {
    fn name(&self) -> &'static str {
        "offer-set"
    }

    fn describe(&self) -> &'static str {
        "completed sets at offer time -> prerequisites"
    }

    fn run(&self, input: &SignalInput) -> Result<Findings> {
        let params = &PARAMS;
        let catalog = input.catalog;
        let mut stats = ObserveStats::default();
        let candidates = prerequisite_candidates(catalog);
        let variants = VariantIndex::new(catalog);
        let mut finishers = FinisherIndex::new(catalog, []);
        // Each quest's views are folded in character order (evidence lists the first characters).
        // A character is whole in its bucket, so the buckets in order give every character in order.
        let mut folds: BTreeMap<QuestId, QuestFold> = BTreeMap::new();
        input.episodes.for_each_bucket(&mut |bucket| {
            let characters: Vec<_> = group_by_character(bucket, &mut stats).into_values().collect();
            for &episode in characters.iter().flatten() {
                finishers.add(episode);
            }
            for chunk in characters.chunks(CHARACTERS_PER_CHUNK) {
                let views: Vec<Vec<CharacterView>> = chunk
                    .par_iter()
                    .map(|episodes| observe_character(episodes, catalog, &candidates, params.completion_tolerance_seconds))
                    .collect();
                let mut views_by_quest: FxHashMap<QuestId, Vec<&CharacterView>> = FxHashMap::default();
                for view in views.iter().flatten() {
                    stats.views += 1;
                    views_by_quest.entry(view.quest_id).or_default().push(view);
                    folds.entry(view.quest_id).or_insert_with(|| QuestFold::new(view.quest_id));
                }
                folds.par_iter_mut().for_each(|(quest_id, fold)| {
                    for view in views_by_quest.get(quest_id).into_iter().flatten() {
                        fold.add(view, catalog, params, &variants);
                    }
                });
            }
            Ok(())
        })?;
        let quests_seen = folds.len();

        let assessments: BTreeMap<QuestId, QuestAssessment> = folds
            .into_par_iter()
            .map(|(quest_id, fold)| (quest_id, fold.finish(catalog, params, &finishers, &variants)))
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
            quests_seen,
            covered.len(),
            candidates.len()
        );
        let summary = json!({
            "generatedAt": input.generated_at,
            "observe": stats,
            "questsSeen": quests_seen,
            "covered": covered.len(),
            "candidates": candidates.len(),
        });
        Ok(Findings {
            input_count: stats.usable_episodes,
            params: to_json(&FileParams {
                tunables: params,
                support: "independent characters",
                // The TS signal's value, kept so both files say the same.
                score: "calibrated lookup, see signals/offer-set/score.ts",
            }),
            covered_quest_ids: covered,
            candidates,
            reports: vec![("offer-set-summary", summary)],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode_store::{Episodes, InMemory, PerCharacter};
    use crate::model::{CatalogQuest, CompletedChange, CompletedSource, CompletedTimeline, QuestEpisode, QuestEventKind, Starters};
    use crate::test_support::{catalog, catalog_quest, event, gossip, make_episode, npc};

    const UNIVERSAL: QuestId = 1;
    const PREREQ: QuestId = 10;
    const QUEST: QuestId = 20;

    /// A character that turns PREREQ in at npc 100 (no catalog finisher: only the traces say where)
    /// and is then offered QUEST there.
    fn takes_quest_after_prereq(offered_prereq_at: f64) -> QuestEpisode {
        QuestEpisode {
            completed: Some(CompletedTimeline {
                source: CompletedSource::Modern,
                initial: vec![UNIVERSAL],
                changes: vec![CompletedChange {
                    t: 101.0,
                    add: Some(vec![PREREQ]),
                    remove: None,
                }],
            }),
            offers: vec![gossip(offered_prereq_at, npc(100), &[PREREQ], &[]), gossip(100.5, npc(100), &[QUEST], &[])],
            quest_events: vec![event(100.0, QuestEventKind::TurnedIn, PREREQ, Some(npc(100)))],
            ..make_episode()
        }
    }

    #[test]
    fn finds_the_same_with_every_character_in_a_bucket_of_its_own() {
        let catalog = catalog([
            catalog_quest(UNIVERSAL),
            catalog_quest(PREREQ),
            CatalogQuest {
                starters: Starters {
                    npcs: vec![100],
                    ..Starters::default()
                },
                ..catalog_quest(QUEST)
            },
        ]);
        let episodes: Vec<QuestEpisode> = (0..12).map(|i| takes_quest_after_prereq(10.0 + f64::from(i))).collect();
        let run = |episodes: &dyn Episodes| {
            OfferSet
                .run(&SignalInput {
                    episodes,
                    catalog: &catalog,
                    generated_at: "2026-11-22T00:00:00.000Z",
                })
                .unwrap()
        };
        let one_bucket = run(&InMemory(&episodes));
        let per_character = run(&PerCharacter(&episodes));

        assert!(!one_bucket.candidates.is_empty(), "the fixture must produce candidates");
        assert_eq!(per_character.candidates, one_bucket.candidates);
        assert_eq!(per_character.covered_quest_ids, one_bucket.covered_quest_ids);
        assert_eq!(per_character.input_count, one_bucket.input_count);
        assert_eq!(per_character.reports, one_bucket.reports);
    }
}
