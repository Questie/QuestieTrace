//! Scores a signal's candidates against the ground truth: precision and recall per field and
//! per tier. This is how signals are tuned: run the signal, `relations score <signal>`, adjust.
//!
//! Precision only counts candidates for quests the tier knows about. Recall only counts truth
//! edges for quests the signal said it covered (`covered_quest_ids`), so a signal is not punished
//! for quests it never saw. "prerequisiteAny" merges preQuestSingle and preQuestGroup on both
//! sides, since telling OR from AND is harder than finding the prerequisite.

use crate::model::{CandidateFile, GroundTruth, QuestId, RelationField, RelationSet, Tier};
use indexmap::IndexMap;
use serde::Serialize;
use std::collections::BTreeSet;

/// A field as scoring sees it: one relation field, or both prerequisite fields as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ScoredField {
    Field(RelationField),
    #[serde(serialize_with = "prerequisite_any")]
    PrerequisiteAny,
}

fn prerequisite_any<S: serde::Serializer>(serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str("prerequisiteAny")
}

impl ScoredField {
    fn all() -> impl Iterator<Item = ScoredField> {
        RelationField::ALL.into_iter().map(ScoredField::Field).chain([ScoredField::PrerequisiteAny])
    }

    fn matches(self, field: RelationField) -> bool {
        match self {
            ScoredField::Field(own) => own == field,
            ScoredField::PrerequisiteAny => matches!(field, RelationField::PreQuestSingle | RelationField::PreQuestGroup),
        }
    }

    fn values_of(self, set: Option<&RelationSet>) -> BTreeSet<QuestId> {
        let Some(set) = set else { return BTreeSet::new() };
        RelationField::ALL.into_iter().filter(|&field| self.matches(field)).flat_map(|field| set.get(&field).into_iter().flatten().copied()).collect()
    }

    fn name(self) -> &'static str {
        match self {
            ScoredField::Field(field) => field.name(),
            ScoredField::PrerequisiteAny => "prerequisiteAny",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldScore {
    pub field: ScoredField,
    /// Whether the signal produced any candidate for this field at all.
    pub emitted: bool,
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub precision: f64,
    pub recall: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TierScore {
    pub tier: Tier,
    /// Candidates whose quest is in this tier.
    pub judged_candidates: usize,
    pub fields: Vec<FieldScore>,
    /// A few false positives, for review.
    pub false_positive_examples: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreReport {
    pub signal: String,
    pub min_score: f64,
    pub tiers: Vec<TierScore>,
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

pub fn score_candidates(file: &CandidateFile, truth: &GroundTruth, min_score: f64) -> ScoreReport {
    let accepted: Vec<_> = file.candidates.iter().filter(|candidate| candidate.score >= min_score).collect();
    let covered: BTreeSet<QuestId> = file.covered_quest_ids.iter().copied().collect();
    let tiers = Tier::ALL
        .into_iter()
        .map(|tier| {
            let relations = truth.tiers.get(tier);
            let in_tier: Vec<_> = accepted.iter().filter(|candidate| relations.contains_key(&candidate.quest_id)).collect();
            let mut false_positive_examples = Vec::new();
            let fields = ScoredField::all()
                .map(|field| {
                    // Several candidates may claim one (quest, target) edge, e.g. for prerequisiteAny.
                    let mut claimed: IndexMap<(QuestId, QuestId), ()> = IndexMap::new();
                    for candidate in in_tier.iter().filter(|candidate| field.matches(candidate.field)) {
                        claimed.insert((candidate.quest_id, candidate.target), ());
                    }
                    let (mut true_positives, mut false_positives) = (0, 0);
                    for &(quest_id, target) in claimed.keys() {
                        if field.values_of(relations.get(&quest_id)).contains(&target) {
                            true_positives += 1;
                        } else {
                            false_positives += 1;
                            if false_positive_examples.len() < 15 {
                                false_positive_examples.push(format!("{}: {quest_id} -> {target}", field.name()));
                            }
                        }
                    }
                    // Recall only looks at covered quests, on both sides of the ratio.
                    let (mut covered_hits, mut false_negatives) = (0, 0);
                    for &quest_id in &covered {
                        for target in field.values_of(relations.get(&quest_id)) {
                            if claimed.contains_key(&(quest_id, target)) {
                                covered_hits += 1;
                            } else {
                                false_negatives += 1;
                            }
                        }
                    }
                    FieldScore {
                        field,
                        emitted: file.candidates.iter().any(|candidate| field.matches(candidate.field)),
                        true_positives,
                        false_positives,
                        false_negatives,
                        precision: ratio(true_positives, true_positives + false_positives),
                        recall: ratio(covered_hits, covered_hits + false_negatives),
                    }
                })
                .collect();
            TierScore {
                tier,
                judged_candidates: in_tier.len(),
                fields,
                false_positive_examples,
            }
        })
        .collect();
    ScoreReport {
        signal: file.signal.clone(),
        min_score,
        tiers,
    }
}

/// Plain-text table of the fields a signal emitted, one block per tier.
pub fn format_score_report(report: &ScoreReport) -> String {
    let mut lines = vec![format!("signal {} (minScore {})", report.signal, report.min_score)];
    for tier in &report.tiers {
        let name = serde_json::to_value(tier.tier).expect("a tier serializes");
        lines.push(format!("  {}: {} judged candidates", name.as_str().unwrap_or_default(), tier.judged_candidates));
        lines.push("    field                     TP     FP     FN   precision  recall".to_owned());
        for field in tier.fields.iter().filter(|field| field.emitted) {
            lines.push(format!(
                "    {:<22} {:>5} {:>6} {:>6}   {:>8.3} {:>7.3}",
                field.field.name(),
                field.true_positives,
                field.false_positives,
                field.false_negatives,
                field.precision,
                field.recall
            ));
        }
        if !tier.false_positive_examples.is_empty() {
            let examples: Vec<_> = tier.false_positive_examples.iter().take(5).cloned().collect();
            lines.push(format!("    false positives e.g. {}", examples.join("; ")));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GroundTruthTiers, RelationCandidate};
    use std::collections::BTreeMap;

    fn candidate(quest_id: QuestId, field: RelationField, target: QuestId, score: f64) -> RelationCandidate {
        RelationCandidate {
            quest_id,
            field,
            target,
            score,
            support: 1,
            contradict: 0,
            evidence: vec![],
            note: None,
        }
    }

    #[test]
    fn precision_counts_tier_quests_and_recall_only_covered_quests() {
        let truth = GroundTruth {
            tiers: GroundTruthTiers {
                authored_forever: BTreeMap::from([
                    (10, RelationSet::from([(RelationField::PreQuestSingle, vec![1, 2])])),
                    (20, RelationSet::from([(RelationField::PreQuestGroup, vec![3])])),
                    (30, RelationSet::new()),
                ]),
                inherited_classic: BTreeMap::new(),
            },
            ..GroundTruth::default()
        };
        let file = CandidateFile {
            signal: "test".into(),
            generated_at: String::new(),
            input_count: 0,
            params: serde_json::Value::Null,
            // Quest 20 is not covered: its missing prerequisite is no false negative.
            covered_quest_ids: vec![10, 30],
            candidates: vec![
                candidate(10, RelationField::PreQuestSingle, 1, 0.9),
                candidate(10, RelationField::PreQuestSingle, 9, 0.9),
                candidate(30, RelationField::PreQuestSingle, 1, 0.1),
                candidate(99, RelationField::PreQuestSingle, 1, 0.9),
            ],
        };
        let report = score_candidates(&file, &truth, 0.5);
        let authored = &report.tiers[0];
        assert_eq!(authored.judged_candidates, 2, "quest 99 is in no tier, quest 30's candidate is below minScore");
        let single = authored.fields.iter().find(|f| f.field == ScoredField::Field(RelationField::PreQuestSingle)).unwrap();
        assert_eq!((single.true_positives, single.false_positives, single.false_negatives), (1, 1, 1));
        assert_eq!((single.precision, single.recall), (0.5, 0.5));
        let any = authored.fields.iter().find(|f| f.field == ScoredField::PrerequisiteAny).unwrap();
        assert_eq!((any.true_positives, any.false_negatives), (1, 1), "prerequisiteAny merges both prerequisite fields");
        assert_eq!(authored.false_positive_examples[0], "preQuestSingle: 10 -> 9");
    }
}
