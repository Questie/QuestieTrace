//! Canonical relation edges. Signals describe one relationship through different fields and
//! directions (breadcrumbs on the target vs breadcrumbForQuestId on the breadcrumb, exclusiveTo on
//! either side, preQuestSingle vs preQuestGroup), so every candidate becomes a claim on one
//! canonical edge and all evidence about the same relationship lines up in one place.

use crate::eligibility::is_forever_quest_id;
use crate::model::{CandidateFile, Evidence, GroundTruth, QuestId, RelationField, RelationSet};
use indexmap::IndexMap;
use rustc_hash::FxHashSet;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

/// - `Prerequisite`: `quest` needs `target` (preQuestSingle or preQuestGroup; OR vs AND is decided
///   per quest during resolution).
/// - `Next`: `quest`.nextQuestInChain = `target`.
/// - `Breadcrumb`: `quest` is a breadcrumb leading to `target` (breadcrumbForQuestId on `quest`,
///   breadcrumbs on `target`).
/// - `Exclusive`: unordered pair, stored with `quest` < `target`; emitted on both sides.
/// - the rest: `quest`.<kind> = `target`, as the field of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EdgeKind {
    Prerequisite,
    Next,
    Breadcrumb,
    Exclusive,
    ParentQuest,
    AvailableUntilCompleted,
    AvailableStartingWith,
    DisabledByQuest,
}

impl EdgeKind {
    /// The order models are fitted and edges scored in.
    pub const ALL: [EdgeKind; 8] = [
        EdgeKind::Prerequisite,
        EdgeKind::Next,
        EdgeKind::Breadcrumb,
        EdgeKind::Exclusive,
        EdgeKind::ParentQuest,
        EdgeKind::AvailableUntilCompleted,
        EdgeKind::AvailableStartingWith,
        EdgeKind::DisabledByQuest,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EdgeKind::Prerequisite => "prerequisite",
            EdgeKind::Next => "next",
            EdgeKind::Breadcrumb => "breadcrumb",
            EdgeKind::Exclusive => "exclusive",
            EdgeKind::ParentQuest => "parentQuest",
            EdgeKind::AvailableUntilCompleted => "availableUntilCompleted",
            EdgeKind::AvailableStartingWith => "availableStartingWith",
            EdgeKind::DisabledByQuest => "disabledByQuest",
        }
    }

    /// Exclusive pairs and breadcrumbs relate two quests and write to both.
    pub fn is_two_sided(self) -> bool {
        matches!(self, EdgeKind::Exclusive | EdgeKind::Breadcrumb)
    }

    /// The field a one-sided kind writes on its `quest` (prerequisites pick theirs per quest).
    pub fn quest_field(self) -> Option<RelationField> {
        match self {
            EdgeKind::Next => Some(RelationField::NextQuestInChain),
            EdgeKind::ParentQuest => Some(RelationField::ParentQuest),
            EdgeKind::AvailableUntilCompleted => Some(RelationField::AvailableUntilCompleted),
            EdgeKind::AvailableStartingWith => Some(RelationField::AvailableStartingWith),
            EdgeKind::DisabledByQuest => Some(RelationField::DisabledByQuest),
            EdgeKind::Prerequisite | EdgeKind::Breadcrumb | EdgeKind::Exclusive => None,
        }
    }
}

/// Identifies an edge: `<kind>:<quest>:<target>` as text, which is how the outputs name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EdgeKey {
    pub kind: EdgeKind,
    pub quest: QuestId,
    pub target: QuestId,
}

impl EdgeKey {
    pub fn new(kind: EdgeKind, quest: QuestId, target: QuestId) -> EdgeKey {
        EdgeKey { kind, quest, target }
    }

    /// Quests whose relations the edge writes. Exclusive pairs and breadcrumbs write both sides.
    pub fn quests(self) -> impl Iterator<Item = QuestId> {
        std::iter::once(self.quest).chain(self.kind.is_two_sided().then_some(self.target))
    }
}

impl fmt::Display for EdgeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.kind.name(), self.quest, self.target)
    }
}

/// Keys order by their text, the tie-break between equally probable edges in resolution.
impl Ord for EdgeKey {
    fn cmp(&self, other: &Self) -> Ordering {
        if self == other {
            return Ordering::Equal;
        }
        self.to_string().cmp(&other.to_string())
    }
}

impl PartialOrd for EdgeKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Serialize for EdgeKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// A source is one signal's statements in one field, `<signal>/<field>`, with mirrored fields
/// folded (breadcrumbs -> breadcrumbForQuestId). The model calibrates each source as one unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Source<'a> {
    pub signal: &'a str,
    pub field: RelationField,
}

impl fmt::Display for Source<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.signal, self.field)
    }
}

impl Serialize for Source<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// One signal's statement about an edge, in the field it used.
#[derive(Clone, Debug)]
pub struct Claim<'a> {
    pub source: Source<'a>,
    /// The candidate's own field (`source.field` is folded).
    pub field: RelationField,
    pub score: f64,
    pub support: u32,
    pub contradict: u32,
    pub evidence: &'a [Evidence],
}

impl<'a> Claim<'a> {
    pub fn signal(&self) -> &'a str {
        self.source.signal
    }
}

#[derive(Clone, Debug)]
pub struct Edge<'a> {
    pub key: EdgeKey,
    pub claims: Vec<Claim<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Domain {
    Forever,
    Classic,
}

/// Maps a candidate's (questId, field, target) onto its canonical edge, plus the field its source
/// is filed under.
pub fn canonicalize(quest_id: QuestId, field: RelationField, target: QuestId) -> (EdgeKey, RelationField) {
    use RelationField as F;
    match field {
        F::PreQuestSingle | F::PreQuestGroup => (EdgeKey::new(EdgeKind::Prerequisite, quest_id, target), field),
        F::NextQuestInChain => (EdgeKey::new(EdgeKind::Next, quest_id, target), field),
        F::BreadcrumbForQuestId => (EdgeKey::new(EdgeKind::Breadcrumb, quest_id, target), field),
        F::Breadcrumbs => (EdgeKey::new(EdgeKind::Breadcrumb, target, quest_id), F::BreadcrumbForQuestId),
        F::ExclusiveTo => (EdgeKey::new(EdgeKind::Exclusive, quest_id.min(target), quest_id.max(target)), field),
        F::ParentQuest => (EdgeKey::new(EdgeKind::ParentQuest, quest_id, target), field),
        F::AvailableUntilCompleted => (EdgeKey::new(EdgeKind::AvailableUntilCompleted, quest_id, target), field),
        F::AvailableStartingWith => (EdgeKey::new(EdgeKind::AvailableStartingWith, quest_id, target), field),
        F::DisabledByQuest => (EdgeKey::new(EdgeKind::DisabledByQuest, quest_id, target), field),
    }
}

/// Folds every candidate into canonical edges, in the order they are first claimed. When one
/// source claims the same edge twice (both sides of a breadcrumb or an exclusive pair), the
/// stronger claim is kept: the second is the same observation written on the other quest, not new
/// evidence.
pub fn collect_edges<'a>(files: impl IntoIterator<Item = &'a CandidateFile>) -> IndexMap<EdgeKey, Edge<'a>> {
    let mut edges: IndexMap<EdgeKey, Edge<'a>> = IndexMap::new();
    for file in files {
        for candidate in &file.candidates {
            if candidate.quest_id == candidate.target {
                continue;
            }
            let (key, source_field) = canonicalize(candidate.quest_id, candidate.field, candidate.target);
            let edge = edges.entry(key).or_insert_with(|| Edge { key, claims: Vec::new() });
            let claim = Claim {
                source: Source {
                    signal: &file.signal,
                    field: source_field,
                },
                field: candidate.field,
                score: candidate.score,
                support: candidate.support,
                contradict: candidate.contradict,
                evidence: &candidate.evidence,
            };
            match edge.claims.iter_mut().find(|other| other.source == claim.source) {
                None => edge.claims.push(claim),
                Some(existing) if claim.score > existing.score => *existing = claim,
                Some(_) => {}
            }
        }
    }
    edges
}

/// Which reference data judges a quest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reference {
    Authored,
    Inherited,
    None,
}

/// Which reference data judges a quest, and whether it is Forever-new. Ground-truth tier
/// membership decides, not the id alone: legacy Classic-Era quests can carry Forever-range ids
/// (65593-65610) and still be inherited data, judged like any Classic quest.
pub struct QuestDomains {
    authored: FxHashSet<QuestId>,
    inherited: FxHashSet<QuestId>,
}

impl QuestDomains {
    pub fn new(truth: &GroundTruth) -> QuestDomains {
        QuestDomains {
            authored: truth.tiers.authored_forever.keys().copied().collect(),
            inherited: truth.tiers.inherited_classic.keys().copied().collect(),
        }
    }

    pub fn reference_of(&self, quest_id: QuestId) -> Reference {
        if self.authored.contains(&quest_id) {
            Reference::Authored
        } else if self.inherited.contains(&quest_id) {
            Reference::Inherited
        } else {
            Reference::None
        }
    }

    pub fn is_forever_new(&self, quest_id: QuestId) -> bool {
        !self.inherited.contains(&quest_id) && (self.authored.contains(&quest_id) || is_forever_quest_id(quest_id))
    }

    pub fn domain_of_quest(&self, quest_id: QuestId) -> Domain {
        if self.is_forever_new(quest_id) {
            Domain::Forever
        } else {
            Domain::Classic
        }
    }

    /// Edges touching a Forever-new quest are Forever edges, everything else Classic. Two-sided
    /// edges count as Forever when either side is Forever-new, since the Forever side is what the
    /// Lua output would change.
    pub fn domain_of(&self, key: EdgeKey) -> Domain {
        if key.quests().any(|quest_id| self.is_forever_new(quest_id)) {
            Domain::Forever
        } else {
            Domain::Classic
        }
    }
}

/// Flattened ground truth: one RelationSet per quest across both tiers (tiers never share a quest).
pub type Truth<'a> = BTreeMap<QuestId, &'a RelationSet>;

pub fn truth_lookup(truth: &GroundTruth) -> Truth<'_> {
    truth
        .tiers
        .authored_forever
        .iter()
        .chain(&truth.tiers.inherited_classic)
        .map(|(&quest_id, set)| (quest_id, set))
        .collect()
}

/// Whether the ground truth judges this edge at all: one of the quests it writes has reference data.
pub fn is_judged(key: EdgeKey, truth: &Truth) -> bool {
    key.quests().any(|quest_id| truth.contains_key(&quest_id))
}

/// Whether the ground truth contains this edge, in either of the fields that can express it.
pub fn truth_has(key: EdgeKey, truth: &Truth) -> bool {
    use RelationField as F;
    let has = |quest_id: QuestId, field: RelationField, value: QuestId| {
        truth
            .get(&quest_id)
            .and_then(|set| set.get(&field))
            .is_some_and(|values| values.contains(&value))
    };
    let EdgeKey { kind, quest, target } = key;
    match kind {
        EdgeKind::Prerequisite => has(quest, F::PreQuestSingle, target) || has(quest, F::PreQuestGroup, target),
        EdgeKind::Breadcrumb => has(quest, F::BreadcrumbForQuestId, target) || has(target, F::Breadcrumbs, quest),
        EdgeKind::Exclusive => has(quest, F::ExclusiveTo, target) || has(target, F::ExclusiveTo, quest),
        _ => kind.quest_field().is_some_and(|field| has(quest, field, target)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GroundTruthTiers, RelationCandidate};

    fn file(signal: &str, candidates: &[(QuestId, RelationField, QuestId, f64)]) -> CandidateFile {
        CandidateFile {
            signal: signal.into(),
            generated_at: String::new(),
            input_count: 0,
            params: serde_json::Value::Null,
            covered_quest_ids: vec![],
            candidates: candidates
                .iter()
                .map(|&(quest_id, field, target, score)| RelationCandidate {
                    quest_id,
                    field,
                    target,
                    score,
                    support: 1,
                    contradict: 0,
                    evidence: vec![],
                    note: None,
                })
                .collect(),
        }
    }

    #[test]
    fn folds_both_sides_of_one_relationship_into_one_edge_with_one_claim_per_source() {
        use RelationField as F;
        let files = [
            file("breadcrumbs", &[(2, F::Breadcrumbs, 1, 0.9), (1, F::BreadcrumbForQuestId, 2, 0.8)]),
            file(
                "cooccurrence",
                &[(5, F::ExclusiveTo, 3, 0.7), (3, F::ExclusiveTo, 5, 0.95), (10, F::PreQuestGroup, 4, 0.5)],
            ),
            file("offer-set", &[(10, F::PreQuestSingle, 4, 0.97)]),
        ];
        let edges = collect_edges(&files);

        let keys: Vec<String> = edges.keys().map(ToString::to_string).collect();
        assert_eq!(keys, ["breadcrumb:1:2", "exclusive:3:5", "prerequisite:10:4"]);
        let breadcrumb = &edges[&EdgeKey::new(EdgeKind::Breadcrumb, 1, 2)].claims;
        assert_eq!(breadcrumb.len(), 1);
        assert_eq!(
            (breadcrumb[0].source.to_string(), breadcrumb[0].score),
            ("breadcrumbs/breadcrumbForQuestId".to_string(), 0.9)
        );
        let exclusive = &edges[&EdgeKey::new(EdgeKind::Exclusive, 3, 5)].claims;
        assert_eq!(exclusive.iter().map(|claim| claim.score).collect::<Vec<_>>(), [0.95]);
        let prerequisite = &edges[&EdgeKey::new(EdgeKind::Prerequisite, 10, 4)].claims;
        let sources: Vec<String> = prerequisite.iter().map(|claim| claim.source.to_string()).collect();
        assert_eq!(sources, ["cooccurrence/preQuestGroup", "offer-set/preQuestSingle"]);
    }

    #[test]
    fn domains_go_by_ground_truth_tier_before_quest_id() {
        // 65593 has a Forever-range id but is filed as inherited Classic data.
        let truth = GroundTruth {
            tiers: GroundTruthTiers {
                authored_forever: BTreeMap::from([(92000, RelationSet::new())]),
                inherited_classic: BTreeMap::from([(500, RelationSet::new()), (65593, RelationSet::new())]),
            },
            ..GroundTruth::default()
        };
        let domains = QuestDomains::new(&truth);

        let judged: Vec<_> = [92000, 92001, 65593, 500]
            .iter()
            .map(|&q| (domains.reference_of(q), domains.is_forever_new(q)))
            .collect();
        assert_eq!(
            judged,
            [
                (Reference::Authored, true),
                (Reference::None, true),
                (Reference::Inherited, false),
                (Reference::Inherited, false)
            ]
        );
        assert_eq!(domains.domain_of(EdgeKey::new(EdgeKind::Exclusive, 500, 92001)), Domain::Forever);
        assert_eq!(domains.domain_of(EdgeKey::new(EdgeKind::Prerequisite, 65593, 92001)), Domain::Classic);
    }

    #[test]
    fn truth_has_accepts_a_relationship_recorded_on_either_side() {
        use RelationField as F;
        let sets = [
            (2, RelationSet::from([(F::Breadcrumbs, vec![1])])),
            (5, RelationSet::from([(F::ExclusiveTo, vec![3])])),
            (10, RelationSet::from([(F::PreQuestGroup, vec![4])])),
        ];
        let truth: Truth = sets.iter().map(|(quest_id, set)| (*quest_id, set)).collect();

        assert!(truth_has(EdgeKey::new(EdgeKind::Breadcrumb, 1, 2), &truth));
        assert!(truth_has(EdgeKey::new(EdgeKind::Exclusive, 3, 5), &truth));
        assert!(truth_has(EdgeKey::new(EdgeKind::Prerequisite, 10, 4), &truth));
        assert!(!truth_has(EdgeKey::new(EdgeKind::Next, 10, 4), &truth));
    }

    #[test]
    fn keys_order_by_their_text() {
        // "prerequisite:123:4" < "prerequisite:12:5": '3' sorts before ':'.
        let short = EdgeKey::new(EdgeKind::Prerequisite, 12, 5);
        let long = EdgeKey::new(EdgeKind::Prerequisite, 123, 4);
        assert!(long < short);
        assert!(EdgeKey::new(EdgeKind::Next, 9, 9) < EdgeKey::new(EdgeKind::Prerequisite, 1, 1));
    }
}
