//! Per-quest views of the resolution: every edge as seen from each quest it writes to (a
//! breadcrumb is breadcrumbForQuestId on one side and breadcrumbs on the other), with its status
//! against the quest's reference data. relations.json and the review both render these.
//!
//! Context-only claims (Wowhead, see [`super::inputs`]) ride along for reviewers: on the edges
//! trace signals claim, and as hints on Forever-new quests where only Wowhead claims something.
//! They are never part of an edge's score or outcome.

use super::crossval::{Contribution, ScoredEdge};
use super::edges::{Claim, Domain, Edge, EdgeKey, EdgeKind, QuestDomains, Reference, Truth};
use super::resolve::{Outcome, Resolution};
use crate::model::{QuestCatalog, QuestId, RelationField, RelationSet};
use indexmap::IndexMap;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

/// Against the quest's reference relations (authored for Forever-new quests, inherited for
/// Classic):
/// - agree: the reference has this target in this field.
/// - new: the reference has nothing in this field (prerequisites: in either list; always the case
///   without reference data).
/// - conflict: the reference has other targets in this field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Agree,
    New,
    Conflict,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Agree => "agree",
            Status::New => "new",
            Status::Conflict => "conflict",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EdgeView<'s> {
    pub key: EdgeKey,
    pub field: RelationField,
    pub target: QuestId,
    pub probability: f64,
    pub held_out: bool,
    pub outcome: Outcome,
    pub because: Option<EdgeKey>,
    /// Only for edges written to the RelationSet.
    pub status: Option<Status>,
    pub claims: &'s [Claim<'s>],
    /// Context-only claims on the same relationship (untrusted, never scored).
    pub context: &'s [Claim<'s>],
    pub contributions: &'s [Contribution<'s>],
}

impl EdgeView<'_> {
    pub fn kind(&self) -> EdgeKind {
        self.key.kind
    }

    /// Written by the resolution, so in relations.json and (Forever-new quests) the Lua output.
    pub fn is_written(&self) -> bool {
        self.outcome.is_written()
    }
}

/// A relationship only context-only sources claim.
#[derive(Clone, Debug)]
pub struct HintView<'s> {
    pub field: RelationField,
    pub target: QuestId,
    pub claims: &'s [Claim<'s>],
}

#[derive(Clone, Debug)]
pub struct QuestView<'s> {
    pub quest_id: QuestId,
    pub name: Option<&'s str>,
    pub domain: Domain,
    /// Which reference data the quest has: authored (Forever-new), inherited (Classic) or none.
    pub reference: Reference,
    pub relations: &'s RelationSet,
    /// Most probable first.
    pub edges: Vec<EdgeView<'s>>,
    /// Reference edges the resolution did not write.
    pub reference_only: Vec<(RelationField, QuestId)>,
    /// Forever-new quests only: what context-only sources claim that no trace signal does.
    pub hints: Vec<HintView<'s>>,
    /// How the prerequisites were read as OR or AND.
    pub note: Option<&'s str>,
}

impl QuestView<'_> {
    /// The quest's resolved values in one field.
    pub fn relation(&self, field: RelationField) -> &[QuestId] {
        self.relations.get(&field).map_or(&[][..], Vec::as_slice)
    }
}

static NO_RELATIONS: RelationSet = RelationSet::new();

const PREREQUISITE_FIELDS: [RelationField; 2] = [RelationField::PreQuestSingle, RelationField::PreQuestGroup];

/// The two prerequisite fields are one slot (Questie ignores preQuestGroup when preQuestSingle is
/// set); every other field is its own.
fn slot(field: RelationField) -> &'static [RelationField] {
    match field {
        RelationField::PreQuestSingle | RelationField::PreQuestGroup => &PREREQUISITE_FIELDS,
        RelationField::ExclusiveTo => &[RelationField::ExclusiveTo],
        RelationField::NextQuestInChain => &[RelationField::NextQuestInChain],
        RelationField::ParentQuest => &[RelationField::ParentQuest],
        RelationField::BreadcrumbForQuestId => &[RelationField::BreadcrumbForQuestId],
        RelationField::Breadcrumbs => &[RelationField::Breadcrumbs],
        RelationField::AvailableUntilCompleted => &[RelationField::AvailableUntilCompleted],
        RelationField::AvailableStartingWith => &[RelationField::AvailableStartingWith],
        RelationField::DisabledByQuest => &[RelationField::DisabledByQuest],
    }
}

/// (quest, field, target) for every quest the edge writes to.
fn perspectives(key: EdgeKey, relations: &std::collections::BTreeMap<QuestId, RelationSet>) -> Vec<(QuestId, RelationField, QuestId)> {
    let EdgeKey { kind, quest, target } = key;
    match kind {
        EdgeKind::Prerequisite => {
            let is_group = relations
                .get(&quest)
                .and_then(|set| set.get(&RelationField::PreQuestGroup))
                .is_some_and(|group| group.contains(&target));
            let field = if is_group {
                RelationField::PreQuestGroup
            } else {
                RelationField::PreQuestSingle
            };
            vec![(quest, field, target)]
        }
        EdgeKind::Breadcrumb => vec![
            (quest, RelationField::BreadcrumbForQuestId, target),
            (target, RelationField::Breadcrumbs, quest),
        ],
        EdgeKind::Exclusive => vec![(quest, RelationField::ExclusiveTo, target), (target, RelationField::ExclusiveTo, quest)],
        _ => vec![(quest, kind.quest_field().expect("a one-sided kind names its field"), target)],
    }
}

fn status_of(field: RelationField, target: QuestId, reference: Option<&RelationSet>) -> Status {
    let values = |name: &RelationField| reference.and_then(|set| set.get(name)).map_or(&[][..], Vec::as_slice);
    // The same target under the other prerequisite list kind is a conflict, not agreement.
    if slot(field).iter().all(|name| values(name).is_empty()) {
        Status::New
    } else if values(&field).contains(&target) {
        Status::Agree
    } else {
        Status::Conflict
    }
}

/// An edge as the views see it: a scored edge, or a chain the resolution derived.
struct Shown<'s> {
    key: EdgeKey,
    probability: f64,
    held_out: bool,
    claims: &'s [Claim<'s>],
    contributions: &'s [Contribution<'s>],
}

pub struct ViewSources<'s> {
    pub scored: &'s [ScoredEdge<'s>],
    pub resolution: &'s Resolution,
    pub context: &'s IndexMap<EdgeKey, Edge<'s>>,
    pub truth: &'s Truth<'s>,
    pub domains: &'s QuestDomains,
    pub catalog: &'s QuestCatalog,
}

/// One view per quest an edge at or above `min_shown` (or a written one) touches, per Forever-new
/// quest with hints, and per authored quest; in the order they are first touched.
pub fn build_quest_views<'s>(sources: &ViewSources<'s>, min_shown: f64) -> IndexMap<QuestId, QuestView<'s>> {
    let ViewSources {
        scored,
        resolution,
        context,
        truth,
        domains,
        catalog,
    } = *sources;
    let mut views: IndexMap<QuestId, QuestView<'s>> = IndexMap::new();
    let view_of = |views: &mut IndexMap<QuestId, QuestView<'s>>, quest_id: QuestId| -> usize {
        let entry = views.entry(quest_id);
        let index = entry.index();
        entry.or_insert_with(|| QuestView {
            quest_id,
            name: catalog.quest(quest_id).and_then(|quest| quest.name.as_deref()),
            domain: domains.domain_of_quest(quest_id),
            reference: domains.reference_of(quest_id),
            relations: resolution.relations.get(&quest_id).unwrap_or(&NO_RELATIONS),
            edges: Vec::new(),
            reference_only: Vec::new(),
            hints: Vec::new(),
            note: resolution.notes.get(&quest_id).map(String::as_str),
        });
        index
    };

    let shown = scored
        .iter()
        .map(|edge| Shown {
            key: edge.key(),
            probability: edge.probability,
            held_out: edge.held_out,
            claims: &edge.edge.claims,
            contributions: &edge.contributions,
        })
        .chain(resolution.derived.iter().map(|edge| Shown {
            key: edge.key,
            probability: edge.probability,
            held_out: false,
            claims: &[],
            contributions: &[],
        }));
    let probability_of: FxHashMap<EdgeKey, f64> = scored.iter().map(|edge| (edge.key(), edge.probability)).collect();
    let mut shown_keys = FxHashSet::default();
    for edge in shown {
        shown_keys.insert(edge.key);
        let Some(decision) = resolution.decisions.get(&edge.key) else { continue };
        let written = decision.outcome.is_written();
        if !written && edge.probability < min_shown {
            continue;
        }
        // A chain implied by a breadcrumb is as certain as the breadcrumb, whatever its own evidence says.
        let probability = match (decision.outcome, decision.because) {
            (Outcome::Derived, Some(because)) => edge.probability.max(probability_of.get(&because).copied().unwrap_or(0.0)),
            _ => edge.probability,
        };
        for (quest_id, field, target) in perspectives(edge.key, &resolution.relations) {
            let index = view_of(&mut views, quest_id);
            views[index].edges.push(EdgeView {
                key: edge.key,
                field,
                target,
                probability,
                held_out: edge.held_out,
                outcome: decision.outcome,
                because: decision.because,
                status: written.then(|| status_of(field, target, truth.get(&quest_id).copied())),
                claims: edge.claims,
                context: context.get(&edge.key).map_or(&[][..], |edge| edge.claims.as_slice()),
                contributions: edge.contributions,
            });
        }
    }

    for edge in context.values().filter(|edge| !shown_keys.contains(&edge.key)) {
        for (quest_id, field, target) in perspectives(edge.key, &resolution.relations) {
            if domains.is_forever_new(quest_id) {
                let index = view_of(&mut views, quest_id);
                views[index].hints.push(HintView {
                    field,
                    target,
                    claims: &edge.claims,
                });
            }
        }
    }

    // Authored quests the resolution says nothing about still get a view, for the authored-only list.
    for &quest_id in sources.truth.keys().filter(|quest_id| domains.reference_of(**quest_id) == Reference::Authored) {
        view_of(&mut views, quest_id);
    }
    for view in views.values_mut() {
        view.edges.sort_by(|a, b| b.probability.total_cmp(&a.probability));
        let Some(reference) = truth.get(&view.quest_id) else { continue };
        for (&field, targets) in reference.iter() {
            let written: FxHashSet<QuestId> = slot(field).iter().flat_map(|&name| view.relation(name).iter().copied()).collect();
            view.reference_only
                .extend(targets.iter().filter(|target| !written.contains(target)).map(|&target| (field, target)));
        }
    }
    views
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::edges::truth_lookup;
    use crate::combine::resolve::{resolve_relations, ResolvableEdge, ResolveOptions};
    use crate::model::{GroundTruth, GroundTruthTiers};
    use std::collections::BTreeMap;

    fn scored(kind: EdgeKind, quest: QuestId, target: QuestId, probability: f64) -> ScoredEdge<'static> {
        ScoredEdge {
            edge: Edge {
                key: EdgeKey::new(kind, quest, target),
                claims: vec![],
            },
            probability,
            label: None,
            held_out: true,
            contributions: vec![],
        }
    }

    #[test]
    fn views_each_side_of_an_edge_with_its_status_against_the_reference() {
        use RelationField::{BreadcrumbForQuestId, Breadcrumbs, NextQuestInChain, PreQuestGroup, PreQuestSingle};
        // Quest 1 is a breadcrumb for 2, as its reference says, which turns 1's weakly claimed chain
        // to 2 into a derived one. Quest 10's prerequisite 3 is written as an OR list where the
        // reference has an AND list. Quest 20 is authored and has no evidence at all.
        let truth = GroundTruth {
            tiers: GroundTruthTiers {
                authored_forever: BTreeMap::from([
                    (1, RelationSet::from([(BreadcrumbForQuestId, vec![2])])),
                    (10, RelationSet::from([(PreQuestGroup, vec![3])])),
                    (20, RelationSet::from([(NextQuestInChain, vec![21])])),
                ]),
                inherited_classic: BTreeMap::new(),
            },
            ..GroundTruth::default()
        };
        let scored = vec![
            scored(EdgeKind::Breadcrumb, 1, 2, 0.95),
            scored(EdgeKind::Next, 1, 2, 0.3),
            scored(EdgeKind::Prerequisite, 10, 3, 0.97),
            scored(EdgeKind::Prerequisite, 10, 4, 0.6),
            scored(EdgeKind::Prerequisite, 10, 5, 0.4),
        ];
        let resolvable: Vec<ResolvableEdge> = scored
            .iter()
            .map(|edge| ResolvableEdge {
                key: edge.key(),
                probability: edge.probability,
                claims: &edge.edge.claims,
            })
            .collect();
        let resolution = resolve_relations(
            &resolvable,
            &ResolveOptions {
                min_score: 0.9,
                clique_min_score: 0.5,
                known_quest: &|_| true,
                family_of: &|quest_id| quest_id.to_string(),
                trusts_and: &|_, _| false,
            },
        );
        let (truth_lookup, domains) = (truth_lookup(&truth), QuestDomains::new(&truth));
        let (context, catalog) = (IndexMap::new(), QuestCatalog::default());
        let sources = ViewSources {
            scored: &scored,
            resolution: &resolution,
            context: &context,
            truth: &truth_lookup,
            domains: &domains,
            catalog: &catalog,
        };

        let views = build_quest_views(&sources, 0.5);

        let edges = |quest_id: QuestId| -> Vec<_> {
            views[&quest_id]
                .edges
                .iter()
                .map(|edge| (edge.field, edge.target, edge.probability, edge.outcome, edge.status))
                .collect()
        };
        // A derived chain is as probable as its breadcrumb; ties keep the scored order.
        assert_eq!(
            edges(1),
            [
                (BreadcrumbForQuestId, 2, 0.95, Outcome::Accepted, Some(Status::Agree)),
                (NextQuestInChain, 2, 0.95, Outcome::Derived, Some(Status::New)),
            ]
        );
        assert_eq!(edges(2), [(Breadcrumbs, 1, 0.95, Outcome::Accepted, Some(Status::New))]);
        // The other prerequisite list kind is a conflict, not agreement; edges below 0.5 are left out.
        assert_eq!(
            edges(10),
            [
                (PreQuestSingle, 3, 0.97, Outcome::Accepted, Some(Status::Conflict)),
                (PreQuestSingle, 4, 0.6, Outcome::BelowThreshold, None),
            ]
        );
        // Prerequisites are one slot, so the AND reference counts as written; 20's chain was never claimed.
        assert!(views[&10].reference_only.is_empty());
        assert_eq!(views[&20].reference_only, [(NextQuestInChain, 21)]);
        // Views come in the order they are first touched: edges, then authored quests without any.
        assert_eq!(views.keys().copied().collect::<Vec<_>>(), [1, 2, 10, 20]);
    }
}
