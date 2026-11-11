//! Turns scored edges into one consistent RelationSet per quest. Every edge ends with exactly one
//! [`Decision`], so the review can say why anything with evidence was left out.
//!
//! Order matters, and follows how much each rule can be trusted:
//!   1. threshold: edges at or above minScore are accepted, the rest wait. Edges touching a quest
//!      the catalog does not know are never written: Questie has no such quest, and a prerequisite
//!      on one would hide the quest for good if it cannot be completed in Forever.
//!   2. breadcrumbs: one target per breadcrumb; a breadcrumb B -> T replaces any "T needs B" and
//!      any exclusivity between B and T, and between their same-name copies, and implies
//!      nextQuestInChain(B) = T (every breadcrumbForQuestId in Questie's Classic and Forever data
//!      agrees, 214 of 215). Copies are parallel variants with the same structure, so a copy of B
//!      is a breadcrumb for T's copies too, never their prerequisite. Where Forever completes all
//!      copies at once (not everywhere: faction copies complete separately), prerequisite signals
//!      otherwise see B's copies as prerequisites of every copy of T. (On the 2026-10 data this
//!      rule dropped 107 edges, 106 of them wrong by the ground truth.)
//!   3. scalar fields: one target per quest, runners-up recorded.
//!   4. ordering cycles: prerequisites, chains and breadcrumbs all order quests; the weakest edge
//!      of each cycle goes. This also settles "B needs T" against a breadcrumb B -> T by score.
//!   5. a quest is never exclusive with a quest it comes before or after (directly or through a
//!      chain): the weakest edge of the pair and that path goes. Otherwise the later quest could
//!      never be taken.
//!   6. exclusive groups: a pair below the threshold is still accepted when both quests are
//!      already in one accepted group, the pair has evidence of its own, and rule 5 allows it;
//!      nothing is inferred from transitivity alone.
//!   7. transitive reduction: X needing P2 and P1, where P2 needs P1, keeps only P2, unless the
//!      direct edge to P1 is the stronger one.
//!   8. OR vs AND: see [`choose_group_kind`].
//!
//! Every rule walks edges in input order (then the derived chains, in the order they were
//! derived), and ties between equally probable edges break on the edge key's text: both decide
//! which edge a rule keeps.

use super::edges::{Claim, EdgeKey, EdgeKind};
use crate::model::{QuestId, RelationField, RelationSet};
use indexmap::IndexMap;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;
use std::collections::BTreeMap;
use std::rc::Rc;

/// What resolution needs of an edge.
#[derive(Clone, Copy, Debug)]
pub struct ResolvableEdge<'e, 'a> {
    pub key: EdgeKey,
    pub probability: f64,
    pub claims: &'e [Claim<'a>],
}

pub struct ResolveOptions<'o> {
    pub min_score: f64,
    /// Lower bar for an exclusive pair inside a group whose other pairs were accepted (rule 6).
    pub clique_min_score: f64,
    /// Whether the catalog (Questie's Forever data) knows the quest.
    pub known_quest: &'o dyn Fn(QuestId) -> bool,
    /// Same-name copies share a family; a list of copies of one quest is always an OR list.
    pub family_of: &'o dyn Fn(QuestId) -> String,
    /// Whether a claim from this source counts as evidence that the quest's prerequisites are ALL
    /// required.
    pub trusts_and: &'o dyn Fn(QuestId, &Claim) -> bool,
}

/// - accepted / clique / derived: written to the RelationSet (clique: rule 6; derived: implied by
///   an accepted breadcrumb, see `because`).
/// - below-threshold: not enough evidence.
/// - unknown-quest: one side is missing from the catalog.
/// - runner-up: a scalar field already took a stronger target (`because`).
/// - breadcrumb: replaced by the breadcrumb `because`.
/// - cycle, conflict, transitive: dropped by rules 4, 5 and 7, in favour of `because`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Accepted,
    Clique,
    Derived,
    BelowThreshold,
    UnknownQuest,
    RunnerUp,
    Breadcrumb,
    Cycle,
    Conflict,
    Transitive,
}

impl Outcome {
    /// Written to the RelationSet, so relations.json and (Forever-new quests) the Lua output.
    pub fn is_written(self) -> bool {
        matches!(self, Outcome::Accepted | Outcome::Clique | Outcome::Derived)
    }

    pub fn name(self) -> &'static str {
        match self {
            Outcome::Accepted => "accepted",
            Outcome::Clique => "clique",
            Outcome::Derived => "derived",
            Outcome::BelowThreshold => "below-threshold",
            Outcome::UnknownQuest => "unknown-quest",
            Outcome::RunnerUp => "runner-up",
            Outcome::Breadcrumb => "breadcrumb",
            Outcome::Cycle => "cycle",
            Outcome::Conflict => "conflict",
            Outcome::Transitive => "transitive",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    pub outcome: Outcome,
    /// The edge that caused the outcome.
    pub because: Option<EdgeKey>,
}

/// A nextQuestInChain edge implied by an accepted breadcrumb that no signal claimed.
#[derive(Clone, Copy, Debug)]
pub struct DerivedEdge {
    pub key: EdgeKey,
    /// The breadcrumb's.
    pub probability: f64,
}

pub struct Resolution {
    pub relations: BTreeMap<QuestId, RelationSet>,
    /// One decision per input edge, plus one per derived edge.
    pub decisions: FxHashMap<EdgeKey, Decision>,
    pub derived: Vec<DerivedEdge>,
    /// Per-quest notes on OR vs AND, for the review.
    pub notes: BTreeMap<QuestId, String>,
}

const SCALAR_KINDS: [EdgeKind; 5] = [
    EdgeKind::Next,
    EdgeKind::ParentQuest,
    EdgeKind::AvailableUntilCompleted,
    EdgeKind::AvailableStartingWith,
    EdgeKind::DisabledByQuest,
];

const ORDERING_KINDS: [EdgeKind; 3] = [EdgeKind::Prerequisite, EdgeKind::Next, EdgeKind::Breadcrumb];

/// Ordering edges point from the earlier quest to the later one.
fn ordering_arc(key: EdgeKey) -> (QuestId, QuestId) {
    if key.kind == EdgeKind::Prerequisite {
        (key.target, key.quest)
    } else {
        (key.quest, key.target)
    }
}

/// Index of an edge in [`Resolver::edges`]: the input edges, then the derived ones.
type Id = usize;

struct Resolver<'e, 'a> {
    edges: Vec<ResolvableEdge<'e, 'a>>,
    /// Input edges only: derived edges are never looked up by key.
    by_key: FxHashMap<EdgeKey, Id>,
    decisions: Vec<Decision>,
}

impl Resolver<'_, '_> {
    fn is_live(&self, id: Id) -> bool {
        self.decisions[id].outcome.is_written()
    }

    /// Live edges of these kinds (all kinds when empty), in order.
    fn live(&self, kinds: &[EdgeKind]) -> Vec<Id> {
        (0..self.edges.len())
            .filter(|&id| (kinds.is_empty() || kinds.contains(&self.edges[id].key.kind)) && self.is_live(id))
            .collect()
    }

    fn drop_edge(&mut self, id: Id, outcome: Outcome, because: Id) {
        self.decisions[id] = Decision {
            outcome,
            because: Some(self.edges[because].key),
        };
    }

    /// The more probable edge, or on a tie the one whose key sorts first.
    fn stronger(&self, a: Id, b: Id) -> bool {
        let (a, b) = (&self.edges[a], &self.edges[b]);
        if a.probability != b.probability {
            a.probability > b.probability
        } else {
            a.key < b.key
        }
    }

    fn weakest_of(&self, ids: &[Id]) -> Id {
        ids.iter()
            .copied()
            .reduce(|a, b| if self.stronger(a, b) { b } else { a })
            .expect("a non-empty list")
    }

    fn strongest_of(&self, ids: &[Id]) -> Id {
        ids.iter()
            .copied()
            .reduce(|a, b| if self.stronger(a, b) { a } else { b })
            .expect("a non-empty list")
    }

    fn keep_best_per_quest(&mut self, ids: &[Id]) {
        let mut best: FxHashMap<QuestId, Id> = FxHashMap::default();
        for &id in ids {
            let quest = self.edges[id].key.quest;
            match best.get(&quest) {
                Some(&current) if !self.stronger(id, current) => {}
                _ => {
                    best.insert(quest, id);
                }
            }
        }
        for &id in ids {
            let winner = best[&self.edges[id].key.quest];
            if winner != id {
                self.drop_edge(id, Outcome::RunnerUp, winner);
            }
        }
    }

    /// Live ordering edges by the quest they lead from, in order.
    fn ordering_graph(&self) -> IndexMap<QuestId, Vec<Id>> {
        let mut outgoing: IndexMap<QuestId, Vec<Id>> = IndexMap::new();
        for id in self.live(&ORDERING_KINDS) {
            outgoing.entry(ordering_arc(self.edges[id].key).0).or_default().push(id);
        }
        outgoing
    }

    /// One cycle among the live ordering edges, as its edges, or None when they form a DAG.
    fn find_cycle(&self) -> Option<Vec<Id>> {
        #[derive(PartialEq)]
        enum State {
            Open,
            Done,
        }
        struct Search<'s, 'e, 'a> {
            resolver: &'s Resolver<'e, 'a>,
            outgoing: IndexMap<QuestId, Vec<Id>>,
            state: FxHashMap<QuestId, State>,
            // path[i] leads from path_nodes[i] to path_nodes[i + 1].
            path: Vec<Id>,
            path_nodes: Vec<QuestId>,
        }
        impl Search<'_, '_, '_> {
            fn visit(&mut self, node: QuestId) -> Option<Vec<Id>> {
                self.state.insert(node, State::Open);
                self.path_nodes.push(node);
                let edges = self.outgoing.get(&node).cloned().unwrap_or_default();
                for id in edges {
                    let to = ordering_arc(self.resolver.edges[id].key).1;
                    match self.state.get(&to) {
                        Some(State::Open) => {
                            let start = self.path_nodes.iter().position(|&n| n == to).expect("an open node is on the path");
                            return Some(self.path[start..].iter().copied().chain([id]).collect());
                        }
                        Some(State::Done) => continue,
                        None => {}
                    }
                    self.path.push(id);
                    if let Some(found) = self.visit(to) {
                        return Some(found);
                    }
                    self.path.pop();
                }
                self.path_nodes.pop();
                self.state.insert(node, State::Done);
                None
            }
        }
        let outgoing = self.ordering_graph();
        let starts: Vec<QuestId> = outgoing.keys().copied().collect();
        let mut search = Search {
            resolver: self,
            outgoing,
            state: FxHashMap::default(),
            path: Vec::new(),
            path_nodes: Vec::new(),
        };
        for node in starts {
            if search.state.contains_key(&node) {
                continue;
            }
            if let Some(found) = search.visit(node) {
                return Some(found);
            }
        }
        None
    }

    /// Edges of a path from `from` to `to` over live ordering edges (breadth-first), if any.
    fn find_path(&self, from: QuestId, to: QuestId, outgoing: &IndexMap<QuestId, Vec<Id>>) -> Option<Vec<Id>> {
        let mut came_by: FxHashMap<QuestId, Option<Id>> = FxHashMap::default();
        came_by.insert(from, None);
        let mut queue = std::collections::VecDeque::from([from]);
        while let Some(node) = queue.pop_front() {
            if node == to {
                let mut path = Vec::new();
                let mut edge = came_by[&to];
                while let Some(id) = edge {
                    path.push(id);
                    edge = came_by[&ordering_arc(self.edges[id].key).0];
                }
                path.reverse();
                return Some(path);
            }
            for &id in outgoing.get(&node).into_iter().flatten() {
                let next = ordering_arc(self.edges[id].key).1;
                if !self.is_live(id) || came_by.contains_key(&next) {
                    continue;
                }
                came_by.insert(next, Some(id));
                queue.push_back(next);
            }
        }
        None
    }

    fn path_between(&self, a: QuestId, b: QuestId, outgoing: &IndexMap<QuestId, Vec<Id>>) -> Option<Vec<Id>> {
        self.find_path(a, b, outgoing).or_else(|| self.find_path(b, a, outgoing))
    }
}

pub fn resolve_relations(input: &[ResolvableEdge], options: &ResolveOptions) -> Resolution {
    let mut r = Resolver {
        edges: input.to_vec(),
        by_key: input.iter().enumerate().map(|(id, edge)| (edge.key, id)).collect(),
        decisions: Vec::with_capacity(input.len()),
    };
    let input_ids = 0..input.len();

    // 1. threshold
    for edge in input {
        let known = (options.known_quest)(edge.key.quest) && (options.known_quest)(edge.key.target);
        let outcome = if !known {
            Outcome::UnknownQuest
        } else if edge.probability >= options.min_score {
            Outcome::Accepted
        } else {
            Outcome::BelowThreshold
        };
        r.decisions.push(Decision { outcome, because: None });
    }

    // 2. breadcrumbs
    let family_of = options.family_of;
    let mut between_families: FxHashMap<(EdgeKind, String, String), Vec<Id>> = FxHashMap::default();
    for id in input_ids.clone() {
        let EdgeKey { kind, quest, target } = r.edges[id].key;
        between_families.entry((kind, family_of(quest), family_of(target))).or_default().push(id);
        if kind == EdgeKind::Exclusive {
            between_families.entry((kind, family_of(target), family_of(quest))).or_default().push(id);
        }
    }
    let breadcrumbs = r.live(&[EdgeKind::Breadcrumb]);
    r.keep_best_per_quest(&breadcrumbs);
    for breadcrumb in r.live(&[EdgeKind::Breadcrumb]) {
        let EdgeKey { quest: b, target: t, .. } = r.edges[breadcrumb].key;
        // The breadcrumb's implied chain competes with any other chain claimed for the same quest.
        let rivals: Vec<Id> = r
            .live(&[EdgeKind::Next])
            .into_iter()
            .filter(|&next| r.edges[next].key.quest == b && r.edges[next].key.target != t)
            .collect();
        if !rivals.is_empty() {
            let strongest = r.strongest_of(&rivals);
            if r.stronger(strongest, breadcrumb) {
                r.drop_edge(breadcrumb, Outcome::RunnerUp, strongest);
                continue;
            }
        }
        for rival in rivals {
            r.drop_edge(rival, Outcome::RunnerUp, breadcrumb);
        }
        // Some chains reuse one name for every step, so copies only count when the two names differ.
        let same_name = family_of(b) == family_of(t);
        let explained = [
            (EdgeKind::Prerequisite, t, b, EdgeKey::new(EdgeKind::Prerequisite, t, b)),
            (EdgeKind::Exclusive, b, t, EdgeKey::new(EdgeKind::Exclusive, b.min(t), b.max(t))),
        ];
        for (kind, from, to, exact) in explained {
            let mut replaced: Vec<Id> = r.by_key.get(&exact).copied().into_iter().collect();
            if !same_name {
                replaced.extend(between_families.get(&(kind, family_of(from), family_of(to))).into_iter().flatten());
            }
            for id in replaced {
                if r.decisions[id].outcome != Outcome::UnknownQuest {
                    r.drop_edge(id, Outcome::Breadcrumb, breadcrumb);
                }
            }
        }
        let next_key = EdgeKey::new(EdgeKind::Next, b, t);
        let next = match r.by_key.get(&next_key) {
            Some(&id) => id,
            None => {
                r.edges.push(ResolvableEdge {
                    key: next_key,
                    probability: r.edges[breadcrumb].probability,
                    claims: &[],
                });
                r.decisions.push(Decision {
                    outcome: Outcome::BelowThreshold,
                    because: None,
                });
                r.edges.len() - 1
            }
        };
        if !r.is_live(next) {
            r.decisions[next] = Decision {
                outcome: Outcome::Derived,
                because: Some(r.edges[breadcrumb].key),
            };
        }
    }

    // 3. scalar fields (a breadcrumb's implied chain already beat its rivals in step 2)
    for kind in SCALAR_KINDS {
        let ids = r.live(&[kind]);
        r.keep_best_per_quest(&ids);
    }

    // 4. ordering cycles
    while let Some(cycle) = r.find_cycle() {
        let (weakest, strongest) = (r.weakest_of(&cycle), r.strongest_of(&cycle));
        r.drop_edge(weakest, Outcome::Cycle, strongest);
    }

    // 5. exclusive vs ordering
    let ordering = r.ordering_graph();
    let mut pairs = r.live(&[EdgeKind::Exclusive]);
    // Like the TS pipeline's `localeCompare` on the keys, where ':' sorts before digits: by quest
    // id text, then target id text.
    pairs.sort_by(|&a, &b| {
        let (a, b) = (&r.edges[a], &r.edges[b]);
        b.probability
            .total_cmp(&a.probability)
            .then_with(|| (a.key.quest.to_string(), a.key.target.to_string()).cmp(&(b.key.quest.to_string(), b.key.target.to_string())))
    });
    for pair in pairs {
        let (quest, target) = (r.edges[pair].key.quest, r.edges[pair].key.target);
        while r.is_live(pair) {
            let Some(path) = r.path_between(quest, target, &ordering) else { break };
            let involved: Vec<Id> = std::iter::once(pair).chain(path).collect();
            let (weakest, strongest) = (r.weakest_of(&involved), r.strongest_of(&involved));
            r.drop_edge(weakest, Outcome::Conflict, strongest);
        }
    }

    // 6. exclusive groups
    let mut group = UnionFind::default();
    for id in r.live(&[EdgeKind::Exclusive]) {
        group.union(r.edges[id].key.quest, r.edges[id].key.target);
    }
    for id in input_ids {
        let edge = r.edges[id];
        if edge.key.kind != EdgeKind::Exclusive || r.decisions[id].outcome != Outcome::BelowThreshold || edge.probability < options.clique_min_score {
            continue;
        }
        let (quest, target) = (edge.key.quest, edge.key.target);
        if group.find(quest) == group.find(target) && r.path_between(quest, target, &ordering).is_none() {
            r.decisions[id] = Decision {
                outcome: Outcome::Clique,
                because: None,
            };
        }
    }

    // 7. transitive reduction
    let mut direct: IndexMap<QuestId, Vec<Id>> = IndexMap::new();
    for id in r.live(&[EdgeKind::Prerequisite]) {
        direct.entry(r.edges[id].key.quest).or_default().push(id);
    }
    let mut reach = Reachability {
        direct: &direct,
        edges: &r.edges,
        memo: FxHashMap::default(),
    };
    let mut transitive = Vec::new();
    for list in direct.values() {
        for &edge in list {
            let target = r.edges[edge].key.target;
            let via = list
                .iter()
                .copied()
                .find(|&other| other != edge && !r.stronger(edge, other) && reach.of(r.edges[other].key.target).contains(&target));
            if let Some(via) = via {
                transitive.push((edge, via));
            }
        }
    }
    for (edge, via) in transitive {
        r.drop_edge(edge, Outcome::Transitive, via);
    }

    // 8. OR vs AND, then the RelationSets
    let mut relations: BTreeMap<QuestId, RelationSet> = BTreeMap::new();
    let mut add = |quest_id: QuestId, field: RelationField, value: QuestId| {
        let values = relations.entry(quest_id).or_default().entry(field).or_default();
        if !values.contains(&value) {
            values.push(value);
            values.sort_unstable();
        }
    };
    let mut prerequisites_of: IndexMap<QuestId, Vec<Id>> = IndexMap::new();
    for id in r.live(&[EdgeKind::Prerequisite]) {
        prerequisites_of.entry(r.edges[id].key.quest).or_default().push(id);
    }
    let mut notes = BTreeMap::new();
    for (&quest_id, ids) in &prerequisites_of {
        let list: Vec<ResolvableEdge> = ids.iter().map(|&id| r.edges[id]).collect();
        let (field, note) = choose_group_kind(&list, options.family_of, &|claim| (options.trusts_and)(quest_id, claim));
        if let Some(note) = note {
            notes.insert(quest_id, note);
        }
        for edge in &list {
            add(quest_id, field, edge.key.target);
        }
    }
    for id in r.live(&[]) {
        let EdgeKey { kind, quest, target } = r.edges[id].key;
        match kind {
            EdgeKind::Prerequisite => {}
            EdgeKind::Exclusive => {
                add(quest, RelationField::ExclusiveTo, target);
                add(target, RelationField::ExclusiveTo, quest);
            }
            EdgeKind::Breadcrumb => {
                add(quest, RelationField::BreadcrumbForQuestId, target);
                add(target, RelationField::Breadcrumbs, quest);
            }
            _ => add(quest, kind.quest_field().expect("a one-sided kind names its field"), target),
        }
    }
    for (quest_id, set) in &relations {
        for (field, values) in set {
            assert!(!field.is_scalar() || values.len() == 1, "resolve: scalar {field} of {quest_id} got {values:?}");
        }
    }

    let derived = (input.len()..r.edges.len())
        .map(|id| DerivedEdge {
            key: r.edges[id].key,
            probability: r.edges[id].probability,
        })
        .collect();
    let decisions = r.edges.iter().zip(&r.decisions).map(|(edge, decision)| (edge.key, *decision)).collect();
    Resolution {
        relations,
        decisions,
        derived,
        notes,
    }
}

/// Prerequisites are an OR list (preQuestSingle) unless every distinct prerequisite (counting
/// same-name copies as one) has a preQuestGroup claim from a source trusted for AND. Measured on
/// the 11 judged quests with two or more accepted prerequisite families (2026-10): this rule got
/// all 11 right, "always OR" 4, "always AND" 7. OR is also the safer mistake: it shows a quest
/// early instead of hiding it.
///
/// AND over a family with several copies is kept as OR: preQuestGroup would demand every copy,
/// and the copies are usually faction variants nobody can all complete.
pub fn choose_group_kind(
    prerequisites: &[ResolvableEdge],
    family_of: &dyn Fn(QuestId) -> String,
    trusts_and: &dyn Fn(&Claim) -> bool,
) -> (RelationField, Option<String>) {
    let mut families: IndexMap<String, Vec<&ResolvableEdge>> = IndexMap::new();
    for edge in prerequisites {
        families.entry(family_of(edge.key.target)).or_default().push(edge);
    }
    if families.len() < 2 {
        return (RelationField::PreQuestSingle, None);
    }
    let all_vote_and = families.values().all(|members| {
        members
            .iter()
            .any(|edge| edge.claims.iter().any(|claim| claim.field == RelationField::PreQuestGroup && trusts_and(claim)))
    });
    if !all_vote_and {
        let note = format!("{} distinct prerequisites without AND evidence for each: kept as OR", families.len());
        return (RelationField::PreQuestSingle, Some(note));
    }
    if families.values().any(|members| members.len() > 1) {
        return (
            RelationField::PreQuestSingle,
            Some("AND evidence, but a prerequisite has same-name copies: kept as OR".to_owned()),
        );
    }
    (
        RelationField::PreQuestGroup,
        Some(format!("AND: each of {} prerequisites has preQuestGroup evidence", families.len())),
    )
}

/// Everything a quest transitively needs, over an acyclic prerequisite graph, memoized.
struct Reachability<'g, 'e, 'a> {
    direct: &'g IndexMap<QuestId, Vec<Id>>,
    edges: &'g [ResolvableEdge<'e, 'a>],
    memo: FxHashMap<QuestId, Rc<FxHashSet<QuestId>>>,
}

impl Reachability<'_, '_, '_> {
    fn of(&mut self, quest_id: QuestId) -> Rc<FxHashSet<QuestId>> {
        if let Some(known) = self.memo.get(&quest_id) {
            return known.clone();
        }
        // Marks the quest as being computed, like the TS memo; the graph is acyclic after rule 4.
        self.memo.insert(quest_id, Rc::default());
        let mut result = FxHashSet::default();
        for &id in self.direct.get(&quest_id).into_iter().flatten() {
            let target = self.edges[id].key.target;
            result.insert(target);
            result.extend(self.of(target).iter().copied());
        }
        let result = Rc::new(result);
        self.memo.insert(quest_id, result.clone());
        result
    }
}

#[derive(Default)]
struct UnionFind {
    parent: FxHashMap<QuestId, QuestId>,
}

impl UnionFind {
    fn find(&mut self, x: QuestId) -> QuestId {
        match self.parent.get(&x) {
            Some(&p) if p != x => {
                let root = self.find(p);
                self.parent.insert(x, root);
                root
            }
            _ => x,
        }
    }

    fn union(&mut self, a: QuestId, b: QuestId) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent.insert(ra, rb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::edges::Source;

    const AND_SOURCE: Source = Source {
        signal: "handoff",
        field: RelationField::PreQuestGroup,
    };
    static AND_CLAIM: [Claim<'static>; 1] = [Claim {
        source: AND_SOURCE,
        field: RelationField::PreQuestGroup,
        score: 0.9,
        support: 1,
        contradict: 0,
        evidence: &[],
    }];

    fn edge(kind: EdgeKind, quest: QuestId, target: QuestId, probability: f64) -> ResolvableEdge<'static, 'static> {
        let (a, b) = if kind == EdgeKind::Exclusive {
            (quest.min(target), quest.max(target))
        } else {
            (quest, target)
        };
        ResolvableEdge {
            key: EdgeKey::new(kind, a, b),
            probability,
            claims: &[],
        }
    }

    fn and_edge(quest: QuestId, target: QuestId) -> ResolvableEdge<'static, 'static> {
        ResolvableEdge {
            claims: &AND_CLAIM,
            ..edge(EdgeKind::Prerequisite, quest, target, 0.95)
        }
    }

    fn resolve(edges: &[ResolvableEdge]) -> Resolution {
        let options = ResolveOptions {
            min_score: 0.9,
            clique_min_score: 0.5,
            known_quest: &|quest_id| quest_id != 404,
            // Quests 1-99 are distinct; 100+ share a name with quest - 100 (same-name copies).
            family_of: &|quest_id| format!("family-{}", quest_id % 100),
            trusts_and: &|_, claim| claim.source == AND_SOURCE,
        };
        resolve_relations(edges, &options)
    }

    fn set(entries: &[(RelationField, &[QuestId])]) -> RelationSet {
        entries.iter().map(|&(field, values)| (field, values.to_vec())).collect()
    }

    fn outcome(result: &Resolution, kind: EdgeKind, quest: QuestId, target: QuestId) -> Outcome {
        result.decisions[&EdgeKey::new(kind, quest, target)].outcome
    }

    fn because(result: &Resolution, kind: EdgeKind, quest: QuestId, target: QuestId) -> Option<EdgeKey> {
        result.decisions[&EdgeKey::new(kind, quest, target)].because
    }

    use EdgeKind::{Breadcrumb, Exclusive, Next, Prerequisite};
    use RelationField::{BreadcrumbForQuestId, Breadcrumbs, ExclusiveTo, NextQuestInChain, PreQuestGroup, PreQuestSingle};

    #[test]
    fn turns_a_breadcrumbs_chain_into_breadcrumb_fields_and_drops_the_prerequisite_and_exclusivity_it_explains() {
        let breadcrumb = edge(Breadcrumb, 1, 2, 0.95);
        let result = resolve(&[breadcrumb, edge(Prerequisite, 2, 1, 0.99), edge(Exclusive, 1, 2, 0.95)]);

        assert_eq!(result.relations[&1], set(&[(BreadcrumbForQuestId, &[2]), (NextQuestInChain, &[2])]));
        assert_eq!(result.relations[&2], set(&[(Breadcrumbs, &[1])]));
        assert_eq!(
            result.decisions[&EdgeKey::new(Prerequisite, 2, 1)],
            Decision {
                outcome: Outcome::Breadcrumb,
                because: Some(breadcrumb.key)
            }
        );
        assert_eq!(
            result.decisions[&EdgeKey::new(Exclusive, 1, 2)],
            Decision {
                outcome: Outcome::Breadcrumb,
                because: Some(breadcrumb.key)
            }
        );
        assert_eq!(outcome(&result, Next, 1, 2), Outcome::Derived);
    }

    #[test]
    fn drops_prerequisites_between_same_name_copies_of_a_breadcrumb_and_of_its_target() {
        // 101 and 102 are copies of 1 and 2; where copies complete together, 102 looks like it needs 101.
        let breadcrumb = edge(Breadcrumb, 1, 2, 0.95);
        let result = resolve(&[breadcrumb, edge(Prerequisite, 102, 101, 0.99), edge(Prerequisite, 102, 7, 0.99)]);

        assert_eq!(outcome(&result, Prerequisite, 102, 101), Outcome::Breadcrumb);
        assert_eq!(because(&result, Prerequisite, 102, 101), Some(breadcrumb.key));
        assert_eq!(result.relations[&102], set(&[(PreQuestSingle, &[7])]));
    }

    #[test]
    fn settles_a_breadcrumb_against_a_stronger_reverse_prerequisite_by_score() {
        // 1 needs 2 contradicts "1 is a breadcrumb for 2": 1 would be hidden before it could be taken.
        let result = resolve(&[edge(Breadcrumb, 1, 2, 0.91), edge(Prerequisite, 1, 2, 0.99)]);

        assert_eq!(result.relations[&1], set(&[(PreQuestSingle, &[2])]));
        assert_eq!(outcome(&result, Breadcrumb, 1, 2), Outcome::Cycle);
    }

    #[test]
    fn lets_a_stronger_rival_chain_on_the_breadcrumb_quest_win_over_the_breadcrumb() {
        let result = resolve(&[edge(Breadcrumb, 1, 2, 0.91), edge(Next, 1, 3, 0.99)]);

        assert_eq!(result.relations[&1], set(&[(NextQuestInChain, &[3])]));
        assert_eq!(outcome(&result, Breadcrumb, 1, 2), Outcome::RunnerUp);
        assert!(result.derived.is_empty());
    }

    #[test]
    fn writes_and_only_when_every_distinct_prerequisite_has_and_evidence() {
        let both = resolve(&[and_edge(10, 1), and_edge(10, 2)]);
        let one = resolve(&[and_edge(10, 1), edge(Prerequisite, 10, 2, 0.95)]);

        assert_eq!(both.relations[&10], set(&[(PreQuestGroup, &[1, 2])]));
        assert_eq!(one.relations[&10], set(&[(PreQuestSingle, &[1, 2])]));
        assert_eq!(one.notes[&10], "2 distinct prerequisites without AND evidence for each: kept as OR");
    }

    #[test]
    fn keeps_same_name_copies_of_one_prerequisite_as_an_or_list_even_with_and_evidence() {
        let copies = resolve(&[and_edge(10, 1), and_edge(10, 101)]);
        let copies_and_other = resolve(&[and_edge(10, 1), and_edge(10, 101), and_edge(10, 2)]);

        assert_eq!(copies.relations[&10], set(&[(PreQuestSingle, &[1, 101])]));
        assert_eq!(copies_and_other.relations[&10], set(&[(PreQuestSingle, &[1, 2, 101])]));
    }

    #[test]
    fn keeps_one_target_per_scalar_field_and_records_the_runner_up() {
        let winner = edge(Next, 1, 3, 0.97);
        let result = resolve(&[edge(Next, 1, 2, 0.95), winner]);

        assert_eq!(result.relations[&1], set(&[(NextQuestInChain, &[3])]));
        assert_eq!(
            result.decisions[&EdgeKey::new(Next, 1, 2)],
            Decision {
                outcome: Outcome::RunnerUp,
                because: Some(winner.key)
            }
        );
    }

    #[test]
    fn breaks_probability_ties_on_the_key_text() {
        // "next:1:12" sorts before "next:1:3" as text, though 3 < 12 as numbers.
        let result = resolve(&[edge(Next, 1, 3, 0.95), edge(Next, 1, 12, 0.95)]);

        assert_eq!(result.relations[&1], set(&[(NextQuestInChain, &[12])]));
    }

    #[test]
    fn writes_exclusive_to_on_both_sides_and_completes_a_group_only_with_the_missing_pairs_own_evidence() {
        let result = resolve(&[
            edge(Exclusive, 1, 2, 0.95),
            edge(Exclusive, 2, 3, 0.95),
            edge(Exclusive, 1, 3, 0.6),
            edge(Exclusive, 3, 4, 0.6),
        ]);

        assert_eq!(result.relations[&1], set(&[(ExclusiveTo, &[2, 3])]));
        assert_eq!(result.relations[&2], set(&[(ExclusiveTo, &[1, 3])]));
        assert_eq!(result.relations[&3], set(&[(ExclusiveTo, &[1, 2])]));
        assert!(!result.relations.contains_key(&4));
        assert_eq!(outcome(&result, Exclusive, 1, 3), Outcome::Clique);
    }

    #[test]
    fn drops_prerequisites_implied_by_an_accepted_chain_unless_the_direct_one_is_stronger() {
        let via_two = edge(Prerequisite, 3, 2, 0.99);
        let reduced = resolve(&[via_two, edge(Prerequisite, 2, 1, 0.99), edge(Prerequisite, 3, 1, 0.95)]);
        let kept = resolve(&[edge(Prerequisite, 3, 2, 0.92), edge(Prerequisite, 2, 1, 0.99), edge(Prerequisite, 3, 1, 0.99)]);

        assert_eq!(reduced.relations[&3], set(&[(PreQuestSingle, &[2])]));
        assert_eq!(
            reduced.decisions[&EdgeKey::new(Prerequisite, 3, 1)],
            Decision {
                outcome: Outcome::Transitive,
                because: Some(via_two.key)
            }
        );
        assert_eq!(kept.relations[&3], set(&[(PreQuestSingle, &[1, 2])]));
    }

    #[test]
    fn breaks_ordering_cycles_at_their_weakest_edge_across_prerequisites_and_chains() {
        // 9 -> 1 leads into the cycle 1 -> 2 -> 3 -> 1 and is the weakest edge, but is not part of the cycle.
        let lead_in = edge(Prerequisite, 1, 9, 0.91);
        let result = resolve(&[lead_in, edge(Prerequisite, 2, 1, 0.99), edge(Prerequisite, 3, 2, 0.98), edge(Next, 3, 1, 0.92)]);

        assert_eq!(outcome(&result, Next, 3, 1), Outcome::Cycle);
        assert_eq!(outcome(&result, Prerequisite, 1, 9), Outcome::Accepted);
        assert_eq!(result.relations[&3], set(&[(PreQuestSingle, &[2])]));
        assert_eq!(result.relations[&2], set(&[(PreQuestSingle, &[1])]));
    }

    #[test]
    fn never_makes_a_quest_both_prerequisite_of_and_exclusive_with_the_same_quest() {
        let prerequisite = edge(Prerequisite, 2, 1, 0.99);
        let result = resolve(&[prerequisite, edge(Exclusive, 1, 2, 0.95)]);

        assert_eq!(result.relations[&2], set(&[(PreQuestSingle, &[1])]));
        assert!(!result.relations.contains_key(&1));
        assert_eq!(
            result.decisions[&EdgeKey::new(Exclusive, 1, 2)],
            Decision {
                outcome: Outcome::Conflict,
                because: Some(prerequisite.key)
            }
        );
    }

    #[test]
    fn never_makes_a_quest_exclusive_with_one_it_follows_through_a_chain() {
        let through_chain = resolve(&[edge(Prerequisite, 2, 1, 0.99), edge(Prerequisite, 3, 2, 0.99), edge(Exclusive, 1, 3, 0.95)]);
        let through_next = resolve(&[edge(Next, 1, 2, 0.97), edge(Exclusive, 1, 2, 0.95)]);

        assert_eq!(outcome(&through_chain, Exclusive, 1, 3), Outcome::Conflict);
        assert_eq!(outcome(&through_next, Exclusive, 1, 2), Outcome::Conflict);
        assert_eq!(through_next.relations[&1], set(&[(NextQuestInChain, &[2])]));
    }

    #[test]
    fn settles_equally_probable_exclusive_pairs_in_the_ts_collation_order() {
        // Two tied pairs share one weak chain edge; whichever pair goes first drops it, in favour of
        // its own path's prerequisite. TS sorts the pairs with localeCompare, where ':' sorts before
        // digits: "12:500" before "123:500" (plain text order would flip it) and "1:12" before "1:3"
        // (numeric order would flip that).
        let text_order = resolve(&[
            edge(Exclusive, 123, 500, 0.95),
            edge(Exclusive, 12, 500, 0.95),
            edge(Next, 500, 7, 0.93),
            edge(Prerequisite, 12, 7, 0.99),
            edge(Prerequisite, 123, 7, 0.99),
        ]);
        let numeric_order = resolve(&[
            edge(Exclusive, 1, 3, 0.95),
            edge(Exclusive, 1, 12, 0.95),
            edge(Next, 1, 5, 0.93),
            edge(Prerequisite, 12, 5, 0.99),
            edge(Prerequisite, 3, 5, 0.99),
        ]);

        assert_eq!(
            text_order.decisions[&EdgeKey::new(Next, 500, 7)],
            Decision {
                outcome: Outcome::Conflict,
                because: Some(EdgeKey::new(Prerequisite, 12, 7))
            }
        );
        assert_eq!(
            numeric_order.decisions[&EdgeKey::new(Next, 1, 5)],
            Decision {
                outcome: Outcome::Conflict,
                because: Some(EdgeKey::new(Prerequisite, 12, 5))
            }
        );
    }

    #[test]
    fn completes_exclusive_groups_only_from_pairs_that_survived_the_conflict_rule() {
        // 2 needs 1, so the pair 1-2 goes, and with it the only link putting 1 and 3 in one group.
        let result = resolve(&[
            edge(Exclusive, 1, 2, 0.95),
            edge(Exclusive, 2, 3, 0.95),
            edge(Exclusive, 1, 3, 0.6),
            edge(Prerequisite, 2, 1, 0.99),
        ]);

        assert_eq!(outcome(&result, Exclusive, 1, 3), Outcome::BelowThreshold);
        assert_eq!(result.relations[&2], set(&[(PreQuestSingle, &[1]), (ExclusiveTo, &[3])]));
        assert!(!result.relations.contains_key(&1));
    }

    #[test]
    fn never_writes_a_relation_to_a_quest_the_catalog_does_not_know() {
        let result = resolve(&[edge(Prerequisite, 2, 404, 0.99)]);

        assert!(result.relations.is_empty());
        assert_eq!(outcome(&result, Prerequisite, 2, 404), Outcome::UnknownQuest);
    }
}
