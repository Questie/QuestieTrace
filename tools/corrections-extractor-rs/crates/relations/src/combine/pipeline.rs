//! The combine pipeline in memory, without reading or writing files: canonical edges from the
//! signals' candidate files ([`super::edges`]), out-of-fold scores ([`super::crossval`]), then one
//! consistent RelationSet per quest ([`super::resolve`]) with the ground-truth-dependent rule
//! inputs measured fold-wise ([`super::foldwise`]). The parent module writes the outputs; the
//! tests run this directly.

use super::crossval::{score_edges, CvGroups, FittedKind, ModelParams, ScoredEdge};
use super::edges::{collect_edges, Claim, Edge, Source};
use super::foldwise::{and_sources_by_quest, FoldContext};
use super::inputs::Inputs;
use super::resolve::{resolve_relations, Resolution, ResolvableEdge, ResolveOptions};
use crate::model::QuestId;

/// Accepted relations: relations.json and the Lua output.
pub const MIN_SCORE: f64 = 0.9;
/// Near misses shown in the review, and the second metrics column.
pub const REVIEW_SCORE: f64 = 0.5;
/// Lower bar for completing an accepted exclusive group (resolution rule 6).
pub const CLIQUE_MIN_SCORE: f64 = 0.5;
/// Lower bar for a prerequisite completing an accepted AND list (resolution rule 6b).
pub const AND_MIN_SCORE: f64 = 0.5;

pub struct Combined<'a> {
    pub scored: Vec<ScoredEdge<'a>>,
    pub fitted: Vec<FittedKind<'a>>,
    /// Resolved at the acceptance threshold: what relations.json and the Lua output contain.
    pub accepted: Resolution,
    /// Resolved at the review threshold, for the second metrics column.
    pub at_review_score: Resolution,
    /// Sources trusted for AND evidence on quests without ground truth.
    pub and_sources: Vec<Source<'a>>,
}

pub fn combine<'a>(inputs: &Inputs<'a>, min_score: f64, params: &ModelParams) -> Combined<'a> {
    let edges: Vec<Edge<'a>> = collect_edges(inputs.files.iter().copied()).into_values().collect();
    let family_of = |quest_id: QuestId| inputs.family_of(quest_id);
    let groups = CvGroups::new(edges.iter().map(|edge| edge.key), &family_of);
    let context = FoldContext {
        truth: &inputs.truth,
        groups: &groups,
        folds: params.folds,
    };
    let and_sources = and_sources_by_quest(&edges, &context);
    let (scored, fitted) = score_edges(&edges, &inputs.truth, &inputs.coverage, &groups, params);

    let resolvable: Vec<ResolvableEdge> = scored
        .iter()
        .map(|edge| ResolvableEdge {
            key: edge.key(),
            probability: edge.probability,
            claims: &edge.edge.claims,
        })
        .collect();
    let known_quest = |quest_id: QuestId| inputs.catalog.quests.contains_key(&quest_id);
    let trusts_and = |quest_id: QuestId, claim: &Claim| and_sources.for_quest(quest_id).contains(&claim.source);
    let resolve = |threshold: f64| {
        let options = ResolveOptions {
            min_score: threshold,
            clique_min_score: CLIQUE_MIN_SCORE,
            and_min_score: AND_MIN_SCORE,
            known_quest: &known_quest,
            family_of: &family_of,
            trusts_and: &trusts_and,
        };
        resolve_relations(&resolvable, &options)
    };
    let accepted = resolve(min_score);
    let at_review_score = resolve(REVIEW_SCORE);
    let and_sources = and_sources.all.iter().copied().collect();
    Combined {
        scored,
        fitted,
        accepted,
        at_review_score,
        and_sources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::crossval::DEFAULT_MODEL_PARAMS;
    use crate::combine::inputs::prepare_inputs;
    use crate::model::{CandidateFile, GroundTruth, GroundTruthTiers, QuestCatalog, RelationCandidate, RelationField, RelationSet};
    use crate::test_support::{catalog, catalog_quest};
    use std::collections::BTreeMap;

    fn file(signal: &str, covered: &[QuestId], candidates: Vec<(QuestId, RelationField, QuestId, f64, u32)>) -> CandidateFile {
        CandidateFile {
            signal: signal.into(),
            generated_at: String::new(),
            input_count: 1,
            params: serde_json::Value::Null,
            covered_quest_ids: covered.to_vec(),
            candidates: candidates
                .into_iter()
                .map(|(quest_id, field, target, score, support)| RelationCandidate {
                    quest_id,
                    field,
                    target,
                    score,
                    support,
                    contradict: 0,
                    evidence: vec![],
                    note: None,
                })
                .collect(),
        }
    }

    /// 40 authored quests 92000+i, each claimed to need 91000+i; every fourth claim is wrong. Five
    /// more quests 93000+i have no reference, so the model fitted on all ground truth scores them.
    fn fixture() -> (QuestCatalog, GroundTruth, Vec<CandidateFile>) {
        use RelationField::PreQuestSingle;
        let mut quests = Vec::new();
        for i in 0..45 {
            for (base, name) in [(92000, "Quest"), (91000, "Step"), (90000, "Other")] {
                quests.push(crate::model::CatalogQuest {
                    name: Some(format!("{name} {i}")),
                    ..catalog_quest(base + i)
                });
            }
        }
        for i in 0..5 {
            quests.push(crate::model::CatalogQuest {
                name: Some(format!("Open {i}")),
                ..catalog_quest(93000 + i)
            });
        }
        let catalog = catalog(quests);
        let truth = GroundTruth {
            tiers: GroundTruthTiers {
                authored_forever: (0..40)
                    .map(|i| {
                        (
                            92000 + i,
                            RelationSet::from([(PreQuestSingle, vec![if i % 4 == 0 { 91999 } else { 91000 + i }])]),
                        )
                    })
                    .collect(),
                inherited_classic: BTreeMap::new(),
            },
            ..GroundTruth::default()
        };
        let all: Vec<QuestId> = catalog.quests.keys().copied().collect();
        let i_f = |i: i64| i as f64;
        let handoff = file(
            "handoff",
            &all,
            (0..40)
                .map(|i| {
                    (
                        92000 + i,
                        PreQuestSingle,
                        91000 + i,
                        if i % 4 == 0 { 0.5 + i_f(i % 3) * 0.05 } else { 0.9 + i_f(i % 3) * 0.03 },
                        (i % 7) as u32 + 1,
                    )
                })
                .chain((0..5).map(|i| (93000 + i, PreQuestSingle, 91000 + i, 0.95, 9)))
                .collect(),
        );
        let offer_set = file(
            "offer-set",
            &all,
            (0..40).filter(|i| i % 3 != 0).map(|i| (92000 + i, PreQuestSingle, 91000 + i, 0.9, 4)).collect(),
        );
        (catalog, truth, vec![handoff, offer_set])
    }

    /// The whole combine step: it learns from the authored quests how far to trust each claim, and
    /// accepts the right ones, on quests it has no reference for too.
    #[test]
    fn accepts_the_claims_that_are_right_and_none_that_are_wrong() {
        let (catalog, truth, files) = fixture();
        let inputs = prepare_inputs(&files, &catalog, &truth).unwrap();

        let combined = combine(&inputs, MIN_SCORE, &DEFAULT_MODEL_PARAMS);

        // Right by the fixture: every claim on an authored quest but each fourth, and the five on
        // quests without a reference, which look like them.
        let right = (0..40)
            .filter(|i| i % 4 != 0)
            .map(|i| (92000 + i, 91000 + i))
            .chain((0..5).map(|i| (93000 + i, 91000 + i)));
        let expected: BTreeMap<QuestId, RelationSet> = right
            .map(|(quest, required)| (quest, RelationSet::from([(RelationField::PreQuestSingle, vec![required])])))
            .collect();
        assert_eq!(combined.accepted.relations, expected);
    }
}
