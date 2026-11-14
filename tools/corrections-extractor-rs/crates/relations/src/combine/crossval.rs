//! Fits one model per edge kind and scores every edge, keeping ground truth out of its own scores.
//!
//! Edges the ground truth judges are scored out of fold: K-fold cross-validation over quest groups
//! (see [`CvGroups`]), so this model never saw the label of an edge it scores. Edges nothing
//! judges (Forever-new quests without authored relations) are scored by the model fitted on all
//! ground truth. The same numbers drive both the held-out metrics and the output.
//!
//! Held out for this model only: the signals calibrated their own score tables on the same ground
//! truth, so a claim's score already carries some of its label. Re-fitting every signal per fold
//! is out of scope; instead [`ModelParams::score_features`] = false fits on the label-free columns
//! alone (claimed, support, contradiction rate, silence) as a sanity check on how optimistic that
//! is.
//!
//! One model per kind covers both domains. Separate Forever and Classic models (shrunk toward the
//! pooled one) and a domain column were both tried and scored worse held-out log-loss on almost
//! every kind; the signals' own scores already carry most domain differences. (Measured 2026-10
//! while Wowhead was still a model input, not re-checked since.)

use super::edges::{is_judged, truth_has, Edge, EdgeKey, EdgeKind, Truth};
use super::features::{Coverage, FeatureColumn, FeatureSpace};
use super::logistic::{fit_logistic, FitOptions, LogisticModel};
use crate::model::QuestId;
use indexmap::IndexMap;
use rustc_hash::FxHashMap;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelParams {
    pub folds: usize,
    pub ridge: f64,
    /// False drops every signal-score column (see the module docs).
    pub score_features: bool,
}

pub const DEFAULT_MODEL_PARAMS: ModelParams = ModelParams {
    folds: 5,
    ridge: 1.0,
    score_features: true,
};

/// Log-odds one signal's columns added to an edge (claims positive or negative, silence usually
/// negative), or the intercept's under the signal name "base".
#[derive(Clone, Debug)]
pub struct Contribution<'a> {
    pub signal: &'a str,
    pub log_odds: f64,
}

#[derive(Clone, Debug)]
pub struct ScoredEdge<'a> {
    pub edge: Edge<'a>,
    pub probability: f64,
    /// Ground-truth label, for edges the ground truth judges.
    pub label: Option<bool>,
    /// True when scored out of fold (judged edges); false when scored by the model fitted on all
    /// ground truth.
    pub held_out: bool,
    pub contributions: Vec<Contribution<'a>>,
}

impl ScoredEdge<'_> {
    pub fn key(&self) -> EdgeKey {
        self.edge.key
    }
}

pub struct FittedKind<'a> {
    pub kind: EdgeKind,
    pub columns: Vec<FeatureColumn<'a>>,
    /// Fitted on every judged edge. An intercept of -inf means the kind was not calibrated.
    pub model: LogisticModel,
    pub judged: usize,
    pub positives: usize,
}

/// FNV-1a over UTF-16 code units: stable across runs and machines, unlike anything seeded, and the
/// same folds as the TS pipeline.
fn hash(text: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for unit in text.encode_utf16() {
        h ^= u32::from(unit);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

pub fn fold_of(group: &str, folds: usize) -> usize {
    hash(group) as usize % folds
}

/// CV groups of quests, and through their `quest` side of edges. A group starts as a quest family
/// (same-name copies, since signals copy evidence between copies) and is joined with the other
/// family of every exclusive pair and breadcrumb: those relate two quests at once, and resolution
/// weighs them against the prerequisites, chains and exclusivity between the same two families, so
/// all of those must be held out together. Prerequisites and chains do not join groups: they link
/// nearly everything (4,941 of 7,667 judged edges in one group on the 2026-10 data, against 179
/// for the largest group this way).
///
/// A group is named by its root family, so which family ends up the root decides the fold: the
/// union order (edges in order, the quest's root under the target's) is part of the contract.
pub struct CvGroups<'f> {
    family_of: &'f dyn Fn(QuestId) -> String,
    root: FxHashMap<String, String>,
}

impl<'f> CvGroups<'f> {
    pub fn new(keys: impl IntoIterator<Item = EdgeKey>, family_of: &'f dyn Fn(QuestId) -> String) -> CvGroups<'f> {
        let mut parent: FxHashMap<String, String> = FxHashMap::default();
        fn find(parent: &FxHashMap<String, String>, family: &str) -> String {
            let mut current = family;
            while let Some(up) = parent.get(current) {
                current = up;
            }
            current.to_owned()
        }
        for key in keys.into_iter().filter(|key| key.kind.is_two_sided()) {
            let (a, b) = (find(&parent, &family_of(key.quest)), find(&parent, &family_of(key.target)));
            if a != b {
                parent.insert(a, b);
            }
        }
        let root = parent.keys().map(|family| (family.clone(), find(&parent, family))).collect();
        CvGroups { family_of, root }
    }

    pub fn group_of(&self, quest_id: QuestId) -> String {
        let family = (self.family_of)(quest_id);
        self.root.get(&family).cloned().unwrap_or(family)
    }
}

/// Splits rows into K folds by group: each group's rows land in exactly one test fold. Returns
/// (train, test) per fold.
pub fn cross_validation_splits<T: Copy>(rows: &[T], group_of_row: impl Fn(T) -> String, folds: usize) -> Vec<(Vec<T>, Vec<T>)> {
    let with_fold: Vec<(T, usize)> = rows.iter().map(|&row| (row, fold_of(&group_of_row(row), folds))).collect();
    let rows_where = |keep: &dyn Fn(usize) -> bool| -> Vec<T> { with_fold.iter().filter(|&&(_, fold)| keep(fold)).map(|&(row, _)| row).collect() };
    (0..folds).map(|fold| (rows_where(&|f| f != fold), rows_where(&|f| f == fold))).collect()
}

/// Below this much ground truth a kind cannot be calibrated: one with 3 judged edges, all true,
/// would accept anything.
const MIN_JUDGED: usize = 10;
const MIN_PER_OUTCOME: usize = 3;

struct Row<'e, 'a> {
    edge: &'e Edge<'a>,
    x: Vec<f64>,
    label: Option<bool>,
}

fn explain<'a>(model: &LogisticModel, columns: &[FeatureColumn<'a>], x: &[f64]) -> Vec<Contribution<'a>> {
    let mut by_signal: IndexMap<&'a str, f64> = IndexMap::new();
    for ((column, weight), &value) in columns.iter().zip(&model.weights).zip(x) {
        if value != 0.0 {
            *by_signal.entry(column.signal).or_insert(0.0) += weight * value;
        }
    }
    std::iter::once(Contribution {
        signal: "base",
        log_odds: model.intercept,
    })
    .chain(by_signal.into_iter().map(|(signal, log_odds)| Contribution { signal, log_odds }))
    .collect()
}

/// Scores every edge: kinds in [`EdgeKind::ALL`] order; within a kind the judged edges fold by
/// fold, then the unjudged ones. Returns the scored edges and one fitted model per kind present.
pub fn score_edges<'a>(
    edges: &[Edge<'a>],
    truth: &Truth,
    coverage: &Coverage<'a>,
    groups: &CvGroups,
    params: &ModelParams,
) -> (Vec<ScoredEdge<'a>>, Vec<FittedKind<'a>>) {
    let mut scored = Vec::new();
    let mut fitted = Vec::new();
    for kind in EdgeKind::ALL {
        let of_kind: Vec<&Edge<'a>> = edges.iter().filter(|edge| edge.key.kind == kind).collect();
        if of_kind.is_empty() {
            continue;
        }
        let space = FeatureSpace::new(&of_kind, coverage, params.score_features);
        let rows: Vec<Row> = of_kind
            .iter()
            .map(|&edge| Row {
                edge,
                x: space.vector(edge),
                label: is_judged(edge.key, truth).then(|| truth_has(edge.key, truth)),
            })
            .collect();
        let judged: Vec<&Row> = rows.iter().filter(|row| row.label.is_some()).collect();
        // An explicit zero prior fixes the width even when a small kind leaves a training fold empty.
        let zero = LogisticModel::zero(space.columns.len());
        let fit = |train: &[&Row]| {
            let positives = train.iter().filter(|row| row.label == Some(true)).count();
            if train.len() < MIN_JUDGED || positives < MIN_PER_OUTCOME || train.len() - positives < MIN_PER_OUTCOME {
                // Without enough ground truth an edge cannot be calibrated, so it scores 0 and is never accepted.
                return LogisticModel {
                    intercept: f64::NEG_INFINITY,
                    weights: zero.weights.clone(),
                };
            }
            let x: Vec<&[f64]> = train.iter().map(|row| row.x.as_slice()).collect();
            let labels: Vec<f64> = train.iter().map(|row| if row.label == Some(true) { 1.0 } else { 0.0 }).collect();
            fit_logistic(&x, &labels, &FitOptions::new(params.ridge, &zero))
        };
        let mut emit = |row: &Row<'_, 'a>, model: &LogisticModel, held_out: bool| {
            scored.push(ScoredEdge {
                edge: row.edge.clone(),
                probability: model.predict(&row.x),
                label: row.label,
                held_out,
                contributions: explain(model, &space.columns, &row.x),
            })
        };

        for (train, test) in cross_validation_splits(&judged, |row| groups.group_of(row.edge.key.quest), params.folds) {
            if test.is_empty() {
                continue;
            }
            let model = fit(&train);
            for row in test {
                emit(row, &model, true);
            }
        }
        let full = fit(&judged);
        for row in rows.iter().filter(|row| row.label.is_none()) {
            emit(row, &full, false);
        }
        fitted.push(FittedKind {
            kind,
            columns: space.columns.clone(),
            model: full,
            judged: judged.len(),
            positives: judged.iter().filter(|row| row.label == Some(true)).count(),
        });
    }
    (scored, fitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::edges::{Claim, Source};
    use crate::model::{RelationField, RelationSet};
    use std::collections::BTreeSet;

    #[test]
    fn splits_put_every_group_in_exactly_one_test_fold_and_never_in_its_own_training_set() {
        let rows: Vec<(usize, usize)> = (0..200).map(|i| (i, i % 37)).collect();
        let splits = cross_validation_splits(&rows, |(_, family)| format!("family-{family}"), 5);

        let mut tested: Vec<usize> = splits.iter().flat_map(|(_, test)| test.iter().map(|&(id, _)| id)).collect();
        tested.sort_unstable();
        assert_eq!(tested, (0..200).collect::<Vec<_>>());
        for (train, test) in &splits {
            let train_families: BTreeSet<usize> = train.iter().map(|&(_, family)| family).collect();
            assert!(test.iter().all(|(_, family)| !train_families.contains(family)));
        }
    }

    #[test]
    fn folds_match_the_ts_pipeline_hash() {
        // Published FNV-1a 32-bit test vectors.
        assert_eq!(hash(""), 0x811c_9dc5);
        assert_eq!(hash("a"), 0xe40c_292c);
        assert_eq!(hash("foobar"), 0xbf9c_f968);
        // Non-ASCII family names hash their UTF-16 code units, as the TS pipeline's hash does (values from Node).
        assert_eq!(hash("name:café"), 0xa9ab_4ea5);
        assert_eq!(hash("name:😀"), 0x19c6_9011);
    }

    #[test]
    fn holds_out_both_quests_of_an_exclusive_pair_or_breadcrumb_with_their_copies_but_not_prerequisite_chains() {
        // 1 and 101 are same-name copies; every other quest is its own family.
        let family_of = |quest_id: QuestId| format!("family-{}", quest_id % 100);
        let keys = [
            EdgeKey::new(EdgeKind::Exclusive, 1, 2),
            EdgeKey::new(EdgeKind::Breadcrumb, 3, 4),
            EdgeKey::new(EdgeKind::Prerequisite, 6, 5),
            EdgeKey::new(EdgeKind::Next, 7, 8),
        ];
        let groups = CvGroups::new(keys, &family_of);

        assert_eq!(groups.group_of(2), groups.group_of(1));
        assert_eq!(groups.group_of(2), groups.group_of(101));
        assert_eq!(groups.group_of(4), groups.group_of(3));
        assert_ne!(groups.group_of(6), groups.group_of(5));
        assert_ne!(groups.group_of(8), groups.group_of(7));
        assert_ne!(groups.group_of(3), groups.group_of(1));
    }

    /// A prerequisite edge claimed by one signal; `score` varies so the model has something to learn.
    fn claimed(kind: EdgeKind, quest: QuestId, target: QuestId, score: f64) -> Edge<'static> {
        Edge {
            key: EdgeKey::new(kind, quest, target),
            claims: vec![Claim {
                source: Source {
                    signal: "handoff",
                    field: RelationField::PreQuestSingle,
                },
                field: RelationField::PreQuestSingle,
                score,
                support: 3,
                contradict: 0,
                evidence: &[],
            }],
        }
    }

    #[test]
    fn scores_a_judged_edge_without_ever_seeing_its_own_label() {
        let edges: Vec<Edge> = (0..60)
            .map(|i| claimed(EdgeKind::Prerequisite, 1000 + i, 1, if i % 2 == 0 { 0.95 } else { 0.4 }))
            .collect();
        let sets: Vec<RelationSet> = (0..60)
            .map(|i| {
                if i % 2 == 0 {
                    RelationSet::from([(RelationField::PreQuestSingle, vec![1])])
                } else {
                    RelationSet::new()
                }
            })
            .collect();
        let truth: Truth = edges.iter().zip(&sets).map(|(edge, set)| (edge.key.quest, set)).collect();
        let empty = RelationSet::new();
        let mut flipped = truth.clone();
        flipped.insert(edges[0].key.quest, &empty);
        let family_of = |quest_id: QuestId| format!("family-{quest_id}");
        let groups = CvGroups::new([], &family_of);
        let probe = |truth: &Truth| {
            let (scored, _) = score_edges(&edges, truth, &Coverage::default(), &groups, &DEFAULT_MODEL_PARAMS);
            let probe = scored.into_iter().find(|edge| edge.key() == edges[0].key).expect("the probe is scored");
            (probe.held_out, probe.probability)
        };

        let (held_out, probability) = probe(&truth);
        assert!(held_out);
        assert!(probability > 0.5, "the probe's fold still learned from the other edges: {probability}");
        assert_eq!(probe(&flipped).1, probability);
    }

    #[test]
    fn never_accepts_a_kind_with_too_little_ground_truth_to_calibrate() {
        // Three judged edges, all true: a fitted intercept alone would accept every new edge of the kind.
        let edges: Vec<Edge> = [5, 6, 7, 8].iter().map(|&quest| claimed(EdgeKind::ParentQuest, quest, 1, 0.99)).collect();
        let set = RelationSet::from([(RelationField::ParentQuest, vec![1])]);
        let truth: Truth = [5, 6, 7].iter().map(|&quest| (quest, &set)).collect();
        let family_of = |quest_id: QuestId| quest_id.to_string();
        let groups = CvGroups::new([], &family_of);

        let (scored, fitted) = score_edges(&edges, &truth, &Coverage::default(), &groups, &DEFAULT_MODEL_PARAMS);
        let unjudged = scored.iter().find(|edge| edge.key().quest == 8).expect("quest 8 is scored");

        assert!(!unjudged.held_out);
        assert_eq!(unjudged.probability, 0.0);
        assert_eq!(fitted[0].model.intercept, f64::NEG_INFINITY);
    }
}
