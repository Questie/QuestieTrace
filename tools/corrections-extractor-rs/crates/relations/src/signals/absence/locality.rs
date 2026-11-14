//! Which suspects are plausible relationship partners for a blocked quest, before any evidence.
//!
//! Questie's relations are local: exclusiveTo partners are nearly always same-name variants
//! (zone, race or faction copies), chains and breadcrumbs hand off at the next quest's giver,
//! prerequisites are turned in where the next quest starts. A blocked moment of a veteran
//! character can have hundreds of consistent suspects (every quest they completed that no
//! offered character had); weighting unrelated ones down lets the few plausible ones carry
//! the blame, while a sharp moment (one changed quest) still convicts an unrelated suspect.

use super::offers::OfferIndex;
use crate::model::{GiverKind, GiverRef, QuestCatalog, QuestId};
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalReason {
    SameName,
    ListedTogether,
    SameGiver,
    Handoff,
    FinisherIsGiver,
}

impl LocalReason {
    pub fn name(self) -> &'static str {
        match self {
            LocalReason::SameName => "sameName",
            LocalReason::ListedTogether => "listedTogether",
            LocalReason::SameGiver => "sameGiver",
            LocalReason::Handoff => "handoff",
            LocalReason::FinisherIsGiver => "finisherIsGiver",
        }
    }

    /// Same-name variants and observed hand-offs; the other reasons are weak ties.
    pub fn is_strong(self) -> bool {
        matches!(self, LocalReason::SameName | LocalReason::Handoff)
    }
}

pub struct Locality<'i> {
    index: &'i OfferIndex<'i>,
    catalog: &'i QuestCatalog,
    givers_of: FxHashMap<QuestId, FxHashSet<GiverRef>>,
    finishers_of: FxHashMap<QuestId, FxHashSet<GiverRef>>,
}

impl<'i> Locality<'i> {
    pub fn new(index: &'i OfferIndex<'i>, catalog: &'i QuestCatalog) -> Self {
        let mut givers_of: FxHashMap<QuestId, FxHashSet<GiverRef>> = FxHashMap::default();
        let mut finishers_of: FxHashMap<QuestId, FxHashSet<GiverRef>> = FxHashMap::default();
        for (&giver, quests) in &index.quests_by_giver {
            for &quest_id in quests {
                givers_of.entry(quest_id).or_default().insert(giver);
            }
        }
        let refs = |kind: GiverKind, ids: &'i [i64]| ids.iter().map(move |&id| GiverRef { kind, id });
        for quest in catalog.quests.values() {
            let givers = givers_of.entry(quest.id).or_default();
            givers.extend(refs(GiverKind::Npc, &quest.starters.npcs).chain(refs(GiverKind::Object, &quest.starters.objects)));
            let finishers = finishers_of.entry(quest.id).or_default();
            finishers.extend(refs(GiverKind::Npc, &quest.finishers.npcs).chain(refs(GiverKind::Object, &quest.finishers.objects)));
        }
        for (&quest_id, givers) in &index.finishers_of {
            finishers_of.entry(quest_id).or_default().extend(givers);
        }
        Locality {
            index,
            catalog,
            givers_of,
            finishers_of,
        }
    }

    /// The first reason the two quests are plausibly related, if any.
    pub fn reason(&self, a: QuestId, b: QuestId) -> Option<LocalReason> {
        let name = |quest_id| self.catalog.quest(quest_id).and_then(|quest| quest.name.as_deref());
        if name(a).is_some_and(|name_a| name(b) == Some(name_a)) {
            return Some(LocalReason::SameName);
        }
        if self.index.listed_together(a, b) {
            return Some(LocalReason::ListedTogether);
        }
        if self.index.hands_off(a, b) || self.index.hands_off(b, a) {
            return Some(LocalReason::Handoff);
        }
        let shared = |x: &FxHashMap<QuestId, FxHashSet<GiverRef>>, p: QuestId, q: QuestId| match (x.get(&p), self.givers_of.get(&q)) {
            (Some(from), Some(to)) => !from.is_disjoint(to),
            _ => false,
        };
        if shared(&self.givers_of, a, b) {
            return Some(LocalReason::SameGiver);
        }
        if shared(&self.finishers_of, a, b) || shared(&self.finishers_of, b, a) {
            return Some(LocalReason::FinisherIsGiver);
        }
        None
    }
}
