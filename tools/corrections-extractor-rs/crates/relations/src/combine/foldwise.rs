//! Resolution inputs that are measured on the ground truth, cross-validated like the edge scores:
//! an edge or quest with ground truth gets the measurement made without its own fold, everything
//! else the measurement on all of it. Otherwise the held-out metrics would grade these choices on
//! the same labels that made them.

use super::crossval::{fold_of, CvGroups};
use super::edges::{is_judged, Edge, EdgeKind, Source, Truth};
use crate::model::{QuestId, RelationField};
use indexmap::{IndexMap, IndexSet};

pub struct FoldContext<'c> {
    pub truth: &'c Truth<'c>,
    /// An edge belongs to its `quest` side's group.
    pub groups: &'c CvGroups<'c>,
    pub folds: usize,
}

/// Sources trusted for AND evidence: on all ground truth, and without each fold.
pub struct AndSources<'a, 'c> {
    pub all: IndexSet<Source<'a>>,
    without_fold: Vec<IndexSet<Source<'a>>>,
    context: &'c FoldContext<'c>,
}

impl<'a> AndSources<'a, '_> {
    /// The sources trusted on this quest: measured without the quest's own fold when the ground
    /// truth judges it.
    pub fn for_quest(&self, quest_id: QuestId) -> &IndexSet<Source<'a>> {
        if self.context.truth.contains_key(&quest_id) {
            &self.without_fold[fold_of(&self.context.groups.group_of(quest_id), self.context.folds)]
        } else {
            &self.all
        }
    }
}

/// Sources whose preQuestGroup claims are mostly right about AND (at least 5 claims, at least half
/// of them in the reference preQuestGroup), in the order they are first met. On Classic alone no
/// source passes, so the choice rests on the authored quests.
pub fn and_sources_by_quest<'a, 'c>(edges: &[Edge<'a>], context: &'c FoldContext<'c>) -> AndSources<'a, 'c> {
    let judged: Vec<&Edge<'a>> = edges
        .iter()
        .filter(|edge| edge.key.kind == EdgeKind::Prerequisite && is_judged(edge.key, context.truth))
        .collect();
    let measure = |items: &mut dyn Iterator<Item = &&Edge<'a>>| -> IndexSet<Source<'a>> {
        let mut tally: IndexMap<Source<'a>, (usize, usize)> = IndexMap::new();
        for edge in items {
            let is_and = context
                .truth
                .get(&edge.key.quest)
                .and_then(|set| set.get(&RelationField::PreQuestGroup))
                .is_some_and(|group| group.contains(&edge.key.target));
            for claim in edge.claims.iter().filter(|claim| claim.field == RelationField::PreQuestGroup) {
                let (right, total) = tally.entry(claim.source).or_insert((0, 0));
                *total += 1;
                if is_and {
                    *right += 1;
                }
            }
        }
        tally
            .into_iter()
            .filter(|&(_, (right, total))| total >= 5 && right as f64 / total as f64 >= 0.5)
            .map(|(source, _)| source)
            .collect()
    };
    let fold_of_edge: Vec<usize> = judged
        .iter()
        .map(|edge| fold_of(&context.groups.group_of(edge.key.quest), context.folds))
        .collect();
    let all = measure(&mut judged.iter());
    let without_fold = (0..context.folds)
        .map(|fold| measure(&mut judged.iter().zip(&fold_of_edge).filter(|&(_, &f)| f != fold).map(|(edge, _)| edge)))
        .collect();
    AndSources { all, without_fold, context }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::edges::{Claim, EdgeKey};
    use crate::model::RelationSet;

    #[test]
    fn never_lets_a_judged_quests_own_labels_make_a_source_trusted_for_that_quest() {
        // Only quest 92001's references show "x" voting AND correctly.
        let source = Source {
            signal: "x",
            field: RelationField::PreQuestGroup,
        };
        let edges: Vec<Edge> = (1..=5)
            .map(|target| Edge {
                key: EdgeKey::new(EdgeKind::Prerequisite, 92001, target),
                claims: vec![Claim {
                    source,
                    field: RelationField::PreQuestGroup,
                    score: 0.9,
                    support: 1,
                    contradict: 0,
                    evidence: &[],
                }],
            })
            .collect();
        let set = RelationSet::from([(RelationField::PreQuestGroup, vec![1, 2, 3, 4, 5])]);
        let truth: Truth = Truth::from([(92001, &set)]);
        let family_of = |quest_id: QuestId| format!("group-{quest_id}");
        let groups = CvGroups::new([], &family_of);
        let context = FoldContext {
            truth: &truth,
            groups: &groups,
            folds: 5,
        };

        let sources = and_sources_by_quest(&edges, &context);

        assert!(sources.all.contains(&source));
        assert!(sources.for_quest(92999).contains(&source));
        assert!(!sources.for_quest(92001).contains(&source));
    }
}
