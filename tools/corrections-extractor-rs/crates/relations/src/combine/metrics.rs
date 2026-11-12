//! Held-out precision/recall of the resolved relations per field and tier, next to the best any
//! single signal manages. Scoring goes through [`crate::score`] so the numbers mean the same as
//! `relations score <signal>`.
//!
//! Both sides are scored over the same quests: the union of every signal's coveredQuestIds. A
//! single signal is therefore also charged for quests only other signals saw, which is the fair
//! question here: what would we get using that signal alone?

use crate::model::{CandidateFile, GroundTruth, QuestId, RelationCandidate, RelationSet, Tier};
use crate::score::{score_candidates, FieldScore, ScoredField};
use crate::text::to_fixed;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const SINGLE_SIGNAL_MIN_SCORES: [f64; 5] = [0.0, 0.5, 0.8, 0.9, 0.95];

/// How far "held out" goes. Written next to the metrics in combine-metrics.json and the review.
pub const METRIC_CAVEATS: [&str; 5] = [
    "Wowhead is context only. Questie's own relation data was scraped from Wowhead Series rows, so Wowhead's agreement with the ground \
     truth is circular. It adds no model column, coverage or silence, takes no part in resolution, and is left out of the \
     best-single-signal comparison.",
    "Held out for the combine model only. Every signal calibrated its own score table on groundtruth.json, so a claim's score already \
     carries some of its label and these metrics are optimistic. labelFree refits the model without any score column (claimed, support, \
     contradiction rate and silence only) as a sanity check; claimed is not fully label-free either, since signals drop candidates below \
     calibrated cut-offs.",
    "Cross-validated like the edge scores: which sources count as AND evidence.",
    "Chosen in-sample on all ground truth: the shape of the OR/AND rule (11 of 11 judged multi-prerequisite quests), suppressing \
     prerequisites between same-name copies of a breadcrumb and its target (106 of the 107 edges it drops are wrong), completing an \
     accepted AND list from below the threshold (resolution rule 6b, with its 0.5 bar for probability and AND claim score), the model \
     form (one pooled model per kind over per-domain models, contradiction rate over count) and ridge 1.",
    "Fixed up front, not tuned: the 0.9 acceptance and 0.5 review thresholds, 0.5 for completing exclusive groups, 5 folds, and at \
     least 10 judged edges with 3 of each outcome before a kind is calibrated.",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SingleSignalScore {
    pub signal: String,
    pub min_score: f64,
    pub score: FieldScore,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsRow {
    pub field: ScoredField,
    /// At the acceptance threshold (what the Lua output uses).
    pub combined: FieldScore,
    /// At the lower review threshold.
    pub review: FieldScore,
    /// Highest recall among single signals at no less than the combined precision, else the most
    /// precise one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_single: Option<SingleSignalScore>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TierMetrics {
    pub tier: Tier,
    pub rows: Vec<MetricsRow>,
}

/// Resolved RelationSets as a CandidateFile, so [`score_candidates`] can judge them.
pub fn relations_as_candidates(name: &str, relations: &BTreeMap<QuestId, RelationSet>, covered_quest_ids: Vec<QuestId>) -> CandidateFile {
    let candidates = relations
        .iter()
        .flat_map(|(&quest_id, set)| {
            set.iter().flat_map(move |(&field, targets)| {
                targets.iter().map(move |&target| RelationCandidate {
                    quest_id,
                    field,
                    target,
                    score: 1.0,
                    support: 0,
                    contradict: 0,
                    evidence: vec![],
                    note: None,
                })
            })
        })
        .collect();
    CandidateFile {
        signal: name.to_owned(),
        generated_at: String::new(),
        input_count: 0,
        params: serde_json::json!({}),
        covered_quest_ids,
        candidates,
    }
}

fn pick_best_single(combined: &FieldScore, options: Vec<SingleSignalScore>) -> Option<SingleSignalScore> {
    let useful: Vec<SingleSignalScore> = options.into_iter().filter(|option| option.score.true_positives > 0).collect();
    let at_precision: Vec<&SingleSignalScore> = useful.iter().filter(|option| option.score.precision >= combined.precision - 0.005).collect();
    // Ties keep the earlier option (files in order, lower thresholds first).
    let best = if at_precision.is_empty() {
        useful.iter().reduce(|a, b| {
            let better = b.score.precision > a.score.precision || (b.score.precision == a.score.precision && b.score.recall > a.score.recall);
            if better {
                b
            } else {
                a
            }
        })
    } else {
        at_precision.into_iter().reduce(|a, b| if b.score.recall > a.score.recall { b } else { a })
    };
    best.cloned()
}

pub fn held_out_metrics(
    accepted: &BTreeMap<QuestId, RelationSet>,
    review: &BTreeMap<QuestId, RelationSet>,
    files: &[&CandidateFile],
    truth: &GroundTruth,
) -> Vec<TierMetrics> {
    let universe: Vec<QuestId> = files
        .iter()
        .flat_map(|file| file.covered_quest_ids.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let combined = score_candidates(&relations_as_candidates("combine", accepted, universe.clone()), truth, 0.0);
    let lower = score_candidates(&relations_as_candidates("combine-review", review, universe.clone()), truth, 0.0);
    let singles: Vec<(&str, f64, crate::score::ScoreReport)> = files
        .iter()
        .flat_map(|&file| {
            let over_universe = CandidateFile {
                covered_quest_ids: universe.clone(),
                ..file.clone()
            };
            SINGLE_SIGNAL_MIN_SCORES.map(|min_score| (file.signal.as_str(), min_score, score_candidates(&over_universe, truth, min_score)))
        })
        .collect();

    combined
        .tiers
        .iter()
        .enumerate()
        .map(|(tier_index, tier)| TierMetrics {
            tier: tier.tier,
            rows: tier
                .fields
                .iter()
                .enumerate()
                .filter(|(_, field)| field.emitted || field.false_negatives > 0)
                .map(|(field_index, field)| {
                    let options = singles
                        .iter()
                        .map(|(signal, min_score, report)| SingleSignalScore {
                            signal: (*signal).to_owned(),
                            min_score: *min_score,
                            score: report.tiers[tier_index].fields[field_index].clone(),
                        })
                        .filter(|option| option.score.emitted)
                        .collect();
                    MetricsRow {
                        field: field.field,
                        combined: field.clone(),
                        review: lower.tiers[tier_index].fields[field_index].clone(),
                        best_single: pick_best_single(field, options),
                    }
                })
                .collect(),
        })
        .collect()
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::AuthoredForever => "authoredForever",
        Tier::InheritedClassic => "inheritedClassic",
    }
}

fn field_name(field: ScoredField) -> &'static str {
    match field {
        ScoredField::Field(field) => field.name(),
        ScoredField::PrerequisiteAny => "prerequisiteAny",
    }
}

fn cell(score: &FieldScore) -> String {
    format!(
        "{} / {} ({}/{}/{})",
        to_fixed(score.precision, 3),
        to_fixed(score.recall, 3),
        score.true_positives,
        score.false_positives,
        score.false_negatives
    )
}

/// Combined precision / recall at the acceptance threshold for two model variants, one row per
/// field and tier.
pub fn format_comparison(full: &[TierMetrics], label_free: &[TierMetrics], min_score: f64) -> String {
    let mut lines = vec![
        format!("| tier | field | full model ≥ {min_score} P / R | without score columns ≥ {min_score} P / R |"),
        "|---|---|---|---|".to_owned(),
    ];
    for (tier, other_tier) in full.iter().zip(label_free) {
        for row in &tier.rows {
            let other = other_tier.rows.iter().find(|candidate| candidate.field == row.field);
            if !row.combined.emitted && !other.is_some_and(|other| other.combined.emitted) {
                continue;
            }
            let other_cell = other.map_or_else(|| "-".to_owned(), |other| cell(&other.combined));
            lines.push(format!(
                "| {} | {} | {} | {other_cell} |",
                tier_name(tier.tier),
                field_name(row.field),
                cell(&row.combined)
            ));
        }
    }
    lines.join("\n")
}

/// Markdown tables, one per tier.
pub fn format_metrics(metrics: &[TierMetrics], min_score: f64, review_score: f64) -> String {
    let mut lines = Vec::new();
    for tier in metrics {
        lines.push(format!("#### {}", tier_name(tier.tier)));
        lines.push(String::new());
        lines.push(format!(
            "| field | combined ≥ {min_score} P / R (TP/FP/FN) | combined ≥ {review_score} P / R | best single signal | its P / R |"
        ));
        lines.push("|---|---|---|---|---|".to_owned());
        for row in &tier.rows {
            let (best, best_score) = match &row.best_single {
                Some(best) => (format!("{} ≥ {}", best.signal, best.min_score), cell(&best.score)),
                None => ("-".to_owned(), "-".to_owned()),
            };
            lines.push(format!(
                "| {} | {} | {} | {best} | {best_score} |",
                field_name(row.field),
                cell(&row.combined),
                cell(&row.review)
            ));
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GroundTruthTiers, RelationField};

    fn truth() -> GroundTruth {
        let single = |target| RelationSet::from([(RelationField::PreQuestSingle, vec![target])]);
        GroundTruth {
            tiers: GroundTruthTiers {
                authored_forever: BTreeMap::from([(92001, single(10)), (92002, single(20)), (92003, single(30))]),
                inherited_classic: BTreeMap::new(),
            },
            ..GroundTruth::default()
        }
    }

    fn signal(name: &str, claims: &[(QuestId, QuestId, f64)]) -> CandidateFile {
        let mut file = relations_as_candidates(name, &BTreeMap::new(), vec![92001, 92002, 92003]);
        file.candidates = claims
            .iter()
            .map(|&(quest_id, target, score)| RelationCandidate {
                quest_id,
                field: RelationField::PreQuestSingle,
                target,
                score,
                support: 1,
                contradict: 0,
                evidence: vec![],
                note: None,
            })
            .collect();
        file
    }

    fn best_single_for(files: &[CandidateFile]) -> Option<SingleSignalScore> {
        let truth = truth();
        let combined = truth.tiers.authored_forever.clone();
        let files: Vec<&CandidateFile> = files.iter().collect();
        let metrics = held_out_metrics(&combined, &combined, &files, &truth);
        let row = metrics[0]
            .rows
            .iter()
            .find(|row| row.field == ScoredField::Field(RelationField::PreQuestSingle))
            .expect("a preQuestSingle row");
        row.best_single.clone()
    }

    #[test]
    fn compares_against_the_single_signal_with_the_most_recall_at_no_less_precision_at_its_own_threshold() {
        let partial = signal("partial", &[(92001, 10, 0.9), (92002, 20, 0.9)]);
        // Complete, but only precise once its 0.3 claim is cut off at 0.5.
        let complete = signal("complete", &[(92001, 10, 0.9), (92002, 20, 0.9), (92003, 30, 0.9), (92003, 99, 0.3)]);

        let best = best_single_for(&[partial, complete]).expect("a best single signal");
        assert_eq!((best.signal.as_str(), best.min_score), ("complete", 0.5));
        assert_eq!((best.score.precision, best.score.recall), (1.0, 1.0));
    }

    #[test]
    fn falls_back_to_the_most_precise_single_signal_when_none_matches_the_combined_precision() {
        let precise = signal("precise", &[(92001, 10, 0.9), (92002, 20, 0.9), (92001, 97, 0.9)]);
        let broad = signal(
            "broad",
            &[
                (92001, 10, 0.9),
                (92002, 20, 0.9),
                (92003, 30, 0.9),
                (92001, 97, 0.9),
                (92002, 98, 0.9),
                (92003, 99, 0.9),
            ],
        );

        assert_eq!(best_single_for(&[precise, broad]).map(|best| best.signal), Some("precise".to_owned()));
    }
}
