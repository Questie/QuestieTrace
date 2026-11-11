//! Which (breadcrumb, target) pairs are worth judging at all, plus the per-quest facts the
//! observation pass needs from every episode at once.
//!
//! A breadcrumb B hands the player over to its target T: B's finisher is T's starter. That link
//! comes from the catalog (B's finishers meet T's starters), from the traces (B turned in at a
//! giver seen offering T), or from the server popping T's dialog right after B's turn-in. Only
//! linked pairs are judged, which keeps the pair count small and unrelated quests out.
//!
//! A quest offered by one of T's own givers is never T's breadcrumb: a breadcrumb sends the player
//! somewhere else (none of Questie's ~215 known breadcrumbs shares a starter with its target), and
//! such pairs are mostly siblings in a hub giver's local chain.
//!
//! Every set here keeps insertion order: observation walks them, and the order pairs are first
//! observed in decides ties in the output, as in the TypeScript pipeline.

use crate::model::{GiverKind, GiverRef, OfferSource, QuestCatalog, QuestEpisode, QuestEventKind, QuestId, SessionTime};
use indexmap::{IndexMap, IndexSet};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};

pub(super) type FxIndexMap<K, V> = IndexMap<K, V, FxBuildHasher>;
pub(super) type FxIndexSet<T> = IndexSet<T, FxBuildHasher>;

#[derive(Clone, Debug)]
pub struct BreadcrumbParams {
    /// A detail this soon after a turn-in at the same giver, with no list in between, is the server's follow-up.
    pub auto_pop_seconds: SessionTime,
    /// A breadcrumb leaving the log this close to the target's accept was dropped by the server.
    pub server_drop_seconds: SessionTime,
    /// Absence evidence needs the character this many levels above the quest's required level.
    pub absence_level_margin: i64,
}

impl Default for BreadcrumbParams {
    fn default() -> Self {
        BreadcrumbParams {
            auto_pop_seconds: 5.0,
            server_drop_seconds: 2.0,
            absence_level_margin: 1,
        }
    }
}

/// A candidate breadcrumb and the quest it would lead to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pair {
    pub breadcrumb: QuestId,
    pub target: QuestId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutoPop {
    pub pair: Pair,
    pub turn_in_t: SessionTime,
    pub detail_t: SessionTime,
    pub giver: Option<GiverRef>,
}

pub struct Links {
    /// Target -> linked candidate breadcrumbs.
    pub by_target: FxIndexMap<QuestId, FxIndexSet<QuestId>>,
    /// Breadcrumb -> linked candidate targets.
    pub by_breadcrumb: FxIndexMap<QuestId, FxIndexSet<QuestId>>,
    /// Giver -> quests it starts, by catalog or by observed offers. Gates absence evidence.
    pub starts_at: FxIndexMap<GiverRef, FxIndexSet<QuestId>>,
    /// Quests some character had in its completed set. Without that, "not completed" means nothing.
    pub ever_completed: FxHashSet<QuestId>,
}

impl Links {
    pub fn breadcrumbs_of(&self, target: QuestId) -> impl Iterator<Item = QuestId> + '_ {
        self.by_target.get(&target).into_iter().flatten().copied()
    }

    pub fn targets_of(&self, breadcrumb: QuestId) -> impl Iterator<Item = QuestId> + '_ {
        self.by_breadcrumb.get(&breadcrumb).into_iter().flatten().copied()
    }

    pub fn started_at(&self, giver: GiverRef) -> impl Iterator<Item = QuestId> + '_ {
        self.starts_at.get(&giver).into_iter().flatten().copied()
    }
}

/// Item "givers" start quests from the bag, so they never hand a player over from one quest to the next.
fn is_place(giver: GiverRef) -> bool {
    giver.kind != GiverKind::Item
}

/// Turn-ins followed by the next quest's dialog at the same giver without a gossip/greeting list in
/// between: the server's own "next quest" pointer, which both chains and breadcrumbs use.
pub fn find_auto_pops(episode: &QuestEpisode, params: &BreadcrumbParams) -> Vec<AutoPop> {
    let list_times: Vec<SessionTime> = episode
        .offers
        .iter()
        .filter(|offer| offer.source != OfferSource::Detail)
        .map(|offer| offer.t)
        .collect();
    let events = &episode.quest_events;
    let mut result = Vec::new();
    for (i, turn_in) in events.iter().enumerate().filter(|(_, event)| event.kind == QuestEventKind::TurnedIn) {
        for next in events[i + 1..].iter().take_while(|next| next.t - turn_in.t <= params.auto_pop_seconds) {
            if next.kind == QuestEventKind::TurnedIn {
                break;
            }
            if next.kind != QuestEventKind::Detail || next.quest_id == turn_in.quest_id {
                continue;
            }
            if matches!((turn_in.giver, next.giver), (Some(a), Some(b)) if a != b) {
                break;
            }
            if list_times.iter().any(|&t| t > turn_in.t && t < next.t) {
                break;
            }
            result.push(AutoPop {
                pair: Pair {
                    breadcrumb: turn_in.quest_id,
                    target: next.quest_id,
                },
                turn_in_t: turn_in.t,
                detail_t: next.t,
                giver: turn_in.giver.or(next.giver),
            });
            break;
        }
    }
    result
}

/// Collects links and global facts in one pass over every episode, because observation needs all
/// of them up front.
pub struct LinkCollector<'p> {
    params: &'p BreadcrumbParams,
    starts_at: FxIndexMap<GiverRef, FxIndexSet<QuestId>>,
    finished_at: FxIndexMap<QuestId, FxIndexSet<GiverRef>>,
    ever_completed: FxHashSet<QuestId>,
    /// Repeatable quests: never breadcrumbs, and the client does not flag them completed.
    repeatable: FxHashSet<QuestId>,
    auto_pop_pairs: FxIndexSet<Pair>,
}

impl<'p> LinkCollector<'p> {
    pub fn new(catalog: &QuestCatalog, params: &'p BreadcrumbParams) -> Self {
        let mut collector = LinkCollector {
            params,
            starts_at: FxIndexMap::default(),
            finished_at: FxIndexMap::default(),
            ever_completed: FxHashSet::default(),
            repeatable: FxHashSet::default(),
            auto_pop_pairs: FxIndexSet::default(),
        };
        let npc = |id| GiverRef { kind: GiverKind::Npc, id };
        let object = |id| GiverRef { kind: GiverKind::Object, id };
        for quest in catalog.quests.values() {
            for giver in quest
                .starters
                .npcs
                .iter()
                .copied()
                .map(npc)
                .chain(quest.starters.objects.iter().copied().map(object))
            {
                collector.starts_at.entry(giver).or_default().insert(quest.id);
            }
            for giver in quest
                .finishers
                .npcs
                .iter()
                .copied()
                .map(npc)
                .chain(quest.finishers.objects.iter().copied().map(object))
            {
                collector.finished_at.entry(quest.id).or_default().insert(giver);
            }
            if quest.special_flags.unwrap_or(0) & 1 != 0 {
                collector.repeatable.insert(quest.id);
            }
        }
        collector
    }

    pub fn add(&mut self, episode: &QuestEpisode) {
        if let Some(completed) = &episode.completed {
            self.ever_completed.extend(&completed.initial);
            self.ever_completed
                .extend(completed.changes.iter().flat_map(|change| change.add.iter().flatten()));
        }
        for offer in &episode.offers {
            for quest in &offer.available {
                if quest.repeatable == Some(true) || quest.frequency.unwrap_or(1) > 1 {
                    self.repeatable.insert(quest.id);
                }
                if let Some(giver) = offer.giver.filter(|&giver| is_place(giver)) {
                    self.starts_at.entry(giver).or_default().insert(quest.id);
                }
            }
        }
        for event in episode.quest_events.iter().filter(|event| event.kind == QuestEventKind::TurnedIn) {
            if let Some(giver) = event.giver.filter(|&giver| is_place(giver)) {
                self.finished_at.entry(event.quest_id).or_default().insert(giver);
            }
        }
        self.auto_pop_pairs.extend(find_auto_pops(episode, self.params).into_iter().map(|pop| pop.pair));
    }

    pub fn finish(self) -> Links {
        let mut started_by: FxHashMap<QuestId, FxHashSet<GiverRef>> = FxHashMap::default();
        for (&giver, quests) in &self.starts_at {
            for &quest in quests {
                started_by.entry(quest).or_default().insert(giver);
            }
        }
        let share_starter = |a: QuestId, b: QuestId| match (started_by.get(&a), started_by.get(&b)) {
            (Some(a), Some(b)) => a.iter().any(|giver| b.contains(giver)),
            _ => false,
        };

        let mut by_target: FxIndexMap<QuestId, FxIndexSet<QuestId>> = FxIndexMap::default();
        let mut by_breadcrumb: FxIndexMap<QuestId, FxIndexSet<QuestId>> = FxIndexMap::default();
        let mut link = |Pair { breadcrumb, target }: Pair| {
            if breadcrumb == target || self.repeatable.contains(&breadcrumb) || self.repeatable.contains(&target) || share_starter(breadcrumb, target) {
                return;
            }
            by_target.entry(target).or_default().insert(breadcrumb);
            by_breadcrumb.entry(breadcrumb).or_default().insert(target);
        };
        for (&breadcrumb, givers) in &self.finished_at {
            for giver in givers {
                for &target in self.starts_at.get(giver).into_iter().flatten() {
                    link(Pair { breadcrumb, target });
                }
            }
        }
        for &pair in &self.auto_pop_pairs {
            link(pair);
        }
        Links {
            by_target,
            by_breadcrumb,
            starts_at: self.starts_at,
            ever_completed: self.ever_completed,
        }
    }
}
