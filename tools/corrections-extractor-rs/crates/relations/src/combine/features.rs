//! Feature vectors for one edge kind's model. Each source (signal + field) that ever claims this
//! kind gets a column for "claimed", plus its score and support as centered log terms when they
//! vary, plus its contradiction rate when it reports any (a rate, not a count: busy chains collect
//! contradictions too, and a raw count learned a positive weight). Each signal also gets a
//! "silent" column: it covered the quest (CandidateFile.coveredQuestIds) and still did not claim
//! the edge, which is the explicit counter-evidence the model can learn to penalize.

use super::edges::{Claim, Edge, Source};
use super::logistic::logit;
use crate::model::QuestId;
use indexmap::IndexMap;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;

/// Per trace signal, the quests it had enough data to judge.
pub type Coverage<'a> = FxHashMap<&'a str, FxHashSet<QuestId>>;

#[derive(Clone, Debug)]
pub struct FeatureColumn<'a> {
    pub name: String,
    /// Signal the column belongs to, for grouping explanations.
    pub signal: &'a str,
}

/// One column of a source's claim.
enum Feature {
    Claimed,
    /// The claim's score logit, minus the source's mean.
    Score {
        center: f64,
    },
    /// log(1 + support), minus the source's mean.
    Support {
        center: f64,
    },
    /// contradict / (support + contradict).
    Contradict,
}

impl Feature {
    fn of(&self, claim: &Claim) -> f64 {
        match *self {
            Feature::Claimed => 1.0,
            Feature::Score { center } => score_logit(claim.score) - center,
            Feature::Support { center } => f64::from(claim.support).ln_1p() - center,
            Feature::Contradict => f64::from(claim.contradict) / f64::from((claim.support + claim.contradict).max(1)),
        }
    }
}

pub struct FeatureSpace<'a, 'c> {
    pub columns: Vec<FeatureColumn<'a>>,
    per_source: Vec<(Source<'a>, Vec<Feature>)>,
    silent_signals: Vec<&'a str>,
    coverage: &'c Coverage<'a>,
}

/// Scores of exactly 0 or 1 would make infinite log-odds.
fn score_logit(score: f64) -> f64 {
    logit(score.clamp(0.005, 0.995))
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

fn varies(values: &[f64]) -> bool {
    values.iter().any(|&value| value != values[0])
}

impl<'a, 'c> FeatureSpace<'a, 'c> {
    /// Built from every edge of the kind (labels are never looked at), so folds share one column
    /// layout. `scores: false` leaves out the score columns: signals calibrated those on the
    /// ground truth.
    pub fn new(edges: &[&Edge<'a>], coverage: &'c Coverage<'a>, scores: bool) -> FeatureSpace<'a, 'c> {
        let mut claims_by_source: IndexMap<Source<'a>, Vec<&Claim<'a>>> = IndexMap::new();
        for claim in edges.iter().flat_map(|edge| &edge.claims) {
            claims_by_source.entry(claim.source).or_default().push(claim);
        }
        // Sources sort by their name, `<signal>/<field>`.
        claims_by_source.sort_by_cached_key(|source, _| source.to_string());

        let mut columns = Vec::new();
        let mut per_source = Vec::new();
        for (source, claims) in &claims_by_source {
            let signal = source.signal;
            let column = |suffix: &str| FeatureColumn {
                name: format!("{source} {suffix}"),
                signal,
            };
            let mut features = vec![Feature::Claimed];
            columns.push(column("claimed"));
            let logits: Vec<f64> = claims.iter().map(|claim| score_logit(claim.score)).collect();
            if scores && varies(&logits) {
                features.push(Feature::Score { center: mean(&logits) });
                columns.push(column("score"));
            }
            let supports: Vec<f64> = claims.iter().map(|claim| f64::from(claim.support).ln_1p()).collect();
            if varies(&supports) {
                features.push(Feature::Support { center: mean(&supports) });
                columns.push(column("support"));
            }
            if claims.iter().any(|claim| claim.contradict > 0) {
                features.push(Feature::Contradict);
                columns.push(column("contradict"));
            }
            per_source.push((*source, features));
        }

        let silent_signals: BTreeSet<&'a str> = claims_by_source
            .keys()
            .map(|source| source.signal)
            .filter(|signal| coverage.contains_key(signal))
            .collect();
        for &signal in &silent_signals {
            columns.push(FeatureColumn {
                name: format!("{signal} silent"),
                signal,
            });
        }

        FeatureSpace {
            columns,
            per_source,
            silent_signals: silent_signals.into_iter().collect(),
            coverage,
        }
    }

    pub fn vector(&self, edge: &Edge) -> Vec<f64> {
        let mut x = Vec::with_capacity(self.columns.len());
        for (source, features) in &self.per_source {
            let claim = edge.claims.iter().find(|claim| claim.source == *source);
            x.extend(features.iter().map(|feature| claim.map_or(0.0, |claim| feature.of(claim))));
        }
        for signal in &self.silent_signals {
            let covered = &self.coverage[signal];
            let silent = !edge.claims.iter().any(|claim| claim.signal() == *signal) && edge.key.quests().all(|quest_id| covered.contains(&quest_id));
            x.push(if silent { 1.0 } else { 0.0 });
        }
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::edges::{EdgeKey, EdgeKind};
    use crate::model::RelationField;

    fn claim(signal: &'static str, score: f64, support: u32, contradict: u32) -> Claim<'static> {
        Claim {
            source: Source {
                signal,
                field: RelationField::PreQuestSingle,
            },
            field: RelationField::PreQuestSingle,
            score,
            support,
            contradict,
            evidence: &[],
        }
    }

    fn edge(kind: EdgeKind, quest: QuestId, target: QuestId, claims: Vec<Claim<'static>>) -> Edge<'static> {
        Edge {
            key: EdgeKey::new(kind, quest, target),
            claims,
        }
    }

    fn coverage(entries: &[(&'static str, &[QuestId])]) -> Coverage<'static> {
        entries.iter().map(|&(signal, quests)| (signal, quests.iter().copied().collect())).collect()
    }

    fn value(space: &FeatureSpace, edge: &Edge, column: &str) -> f64 {
        space.vector(edge)[space
            .columns
            .iter()
            .position(|c| c.name == column)
            .unwrap_or_else(|| panic!("no column {column}"))]
    }

    #[test]
    fn adds_score_and_contradiction_columns_only_where_a_source_varies_and_marks_silent_covering_signals() {
        let edges = [
            edge(EdgeKind::Prerequisite, 1, 5, vec![claim("a", 0.9, 4, 1)]),
            edge(EdgeKind::Prerequisite, 2, 6, vec![claim("a", 0.6, 4, 0), claim("b", 0.8, 4, 0)]),
            edge(EdgeKind::Prerequisite, 3, 7, vec![claim("b", 0.8, 4, 0)]),
        ];
        let edge_refs: Vec<&Edge> = edges.iter().collect();
        // a covers quest 1 only; b covers 1 and 2. Nobody covers 3.
        let coverage = coverage(&[("a", &[1]), ("b", &[1, 2])]);
        let space = FeatureSpace::new(&edge_refs, &coverage, true);

        let names: Vec<&str> = space.columns.iter().map(|column| column.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "a/preQuestSingle claimed",
                "a/preQuestSingle score",
                "a/preQuestSingle contradict",
                "b/preQuestSingle claimed",
                "a silent",
                "b silent"
            ]
        );
        assert_eq!(value(&space, &edges[0], "a/preQuestSingle contradict"), 0.2);
        assert_eq!(value(&space, &edges[0], "a silent"), 0.0);
        assert_eq!(value(&space, &edges[0], "b silent"), 1.0);
        assert_eq!(value(&space, &edges[2], "a silent"), 0.0);
        assert_eq!(value(&space, &edges[2], "b silent"), 0.0);
        assert_eq!(value(&space, &edges[2], "b/preQuestSingle claimed"), 1.0);
        let label_free = FeatureSpace::new(&edge_refs, &coverage, false);
        assert!(label_free.columns.iter().all(|column| column.name != "a/preQuestSingle score"));
    }

    #[test]
    fn counts_a_two_sided_edge_as_covered_only_when_the_signal_covered_both_quests() {
        let pairs = [
            edge(EdgeKind::Exclusive, 1, 2, vec![claim("a", 0.9, 4, 0)]),
            edge(EdgeKind::Exclusive, 1, 3, vec![claim("a", 0.9, 4, 0)]),
            edge(EdgeKind::Exclusive, 1, 4, vec![claim("b", 0.9, 4, 0)]),
        ];
        let pair_refs: Vec<&Edge> = pairs.iter().collect();
        let coverage = coverage(&[("a", &[1, 2, 3]), ("b", &[1, 2])]);
        let space = FeatureSpace::new(&pair_refs, &coverage, true);

        assert_eq!(value(&space, &pairs[0], "b silent"), 1.0);
        assert_eq!(value(&space, &pairs[1], "b silent"), 0.0);
        assert_eq!(value(&space, &pairs[2], "a silent"), 0.0);
    }
}
