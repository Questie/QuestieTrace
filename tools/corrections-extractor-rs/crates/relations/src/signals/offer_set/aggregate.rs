//! Folds every character's view of quest X into per-candidate counts. A candidate P stays
//! "alive" while (almost) every character had it completed at every sighting of X; the other
//! counts measure how specific P is to X, because quests everyone finishes early survive too.
//!
//! Views are added to a [`QuestFold`] one at a time, in character order, as characters are
//! observed, so the views themselves never have to be kept. The fold keeps only what the
//! assessment reads: per candidate its counts and the views it survived in, and only while it can
//! still end up alive or in an OR group; per view only the facts the level gate decides on, which
//! [`QuestFold::finish`] applies once every view is in (without a catalog level, the gate is the
//! lowest level any character first saw the quest at).

use super::observe::{CharacterView, EarlierVisit};
use super::variants::VariantIndex;
use super::Params;
use crate::eligibility::{class_allowed, race_allowed};
use crate::model::{Evidence, GiverKind, GiverRef, PlayerContext, QuestCatalog, QuestEpisode, QuestEventKind, QuestId};
use crate::text::to_fixed;
use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::cmp::Reverse;
use std::collections::BTreeMap;

const MAX_EVIDENCE: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateStats {
    pub target: QuestId,
    /// Characters that had the target completed at every sighting of the quest.
    pub support: u32,
    /// Characters that saw the quest at least once without the target completed.
    pub contradict: u32,
    /// Characters that completed the target earlier in the session of their first sighting,
    /// within `fresh_seconds`.
    pub fresh: u32,
    /// ...and within `immediate_seconds` (turned in, next quest offered right away).
    pub immediate: u32,
    /// Characters for which the target was the most recently completed survivor (within
    /// `fresh_seconds`) before the first sighting.
    pub latest: u32,
    /// Characters that completed the target after arriving at the quest's giver, at a level that
    /// allowed the quest.
    pub pending: u32,
    /// Characters that already had the target completed when they first reached the quest's giver.
    pub background: u32,
    /// The target is turned in (per catalog or traces) at one of the quest's starters or the giver
    /// it was seen at. Only set on alive candidates.
    pub link: bool,
    pub evidence: Vec<Evidence>,
}

impl CandidateStats {
    fn new(target: QuestId) -> Self {
        CandidateStats {
            target,
            support: 0,
            contradict: 0,
            fresh: 0,
            immediate: 0,
            latest: 0,
            pending: 0,
            background: 0,
            link: false,
            evidence: Vec::new(),
        }
    }
}

/// Audience-exclusive same-name variants that together covered every character although no
/// single member did.
#[derive(Clone, Debug, PartialEq)]
pub struct OrGroup {
    /// Ascending.
    pub members: Vec<QuestId>,
    /// Summed over members, targeting the first member; support counts characters with any member
    /// at every sighting.
    pub stats: CandidateStats,
}

#[derive(Debug)]
pub struct QuestAssessment {
    pub quest_id: QuestId,
    /// Independent characters with at least one sighting.
    pub characters: u32,
    /// Characters that first saw the quest right after reaching its required level: for them the
    /// quest may have unlocked by levelling, so whatever they completed just before proves little.
    pub level_unlocked: u32,
    /// Per character whose last complete list from the giver (before first seeing the quest) came
    /// at a level that allowed it: the survivors not completed yet at that list. The list did not
    /// offer the quest, so at least one of them (or something untracked) was still required.
    pub blocked_lists: Vec<Vec<QuestId>>,
    /// Candidates with at most `max_contradictions` contradictions, by ascending target.
    pub alive: Vec<CandidateStats>,
    pub or_groups: Vec<OrGroup>,
}

impl QuestAssessment {
    pub fn alive_stats(&self, target: QuestId) -> Option<&CandidateStats> {
        self.alive
            .binary_search_by_key(&target, |stats| stats.target)
            .ok()
            .map(|index| &self.alive[index])
    }
}

/// Where each quest is turned in: the catalog's finishers plus the givers traces saw it turned in at.
pub struct FinisherIndex<'a> {
    catalog: &'a QuestCatalog,
    turn_ins: FxHashMap<QuestId, FxHashSet<GiverRef>>,
}

impl<'a> FinisherIndex<'a> {
    pub fn new<'e>(catalog: &'a QuestCatalog, episodes: impl IntoIterator<Item = &'e QuestEpisode>) -> Self {
        let mut turn_ins: FxHashMap<QuestId, FxHashSet<GiverRef>> = FxHashMap::default();
        for episode in episodes {
            for event in &episode.quest_events {
                if let Some(giver) = event
                    .giver
                    .filter(|giver| event.kind == QuestEventKind::TurnedIn && giver.kind != GiverKind::Item)
                {
                    turn_ins.entry(event.quest_id).or_default().insert(giver);
                }
            }
        }
        FinisherIndex { catalog, turn_ins }
    }

    /// Whether `quest_id` is turned in at any of `givers`.
    fn turned_in_at_any(&self, quest_id: QuestId, givers: &FxHashSet<GiverRef>) -> bool {
        let catalog = self.catalog.quest(quest_id).map(|quest| &quest.finishers);
        let npcs = catalog
            .into_iter()
            .flat_map(|finishers| &finishers.npcs)
            .map(|&id| GiverRef { kind: GiverKind::Npc, id });
        let objects = catalog
            .into_iter()
            .flat_map(|finishers| &finishers.objects)
            .map(|&id| GiverRef { kind: GiverKind::Object, id });
        let traced = self.turn_ins.get(&quest_id).into_iter().flatten().copied();
        npcs.chain(objects).chain(traced).any(|giver| givers.contains(&giver))
    }
}

fn describe_sighting(view: &CharacterView) -> String {
    let at = view.giver.map(|giver| format!(" at {}", giver.label())).unwrap_or_default();
    let level = view.level.map_or_else(|| "?".to_string(), |level| level.to_string());
    format!("{} first seen ({}){at}, level {level}", view.quest_id, view.kind.name())
}

/// The race and class a character plays, which decide the quests it could ever take.
type Audience = (Option<i64>, Option<i64>);

fn can_take(catalog: &QuestCatalog, target: QuestId, (race_id, class_id): Audience) -> bool {
    let quest = catalog.quest(target);
    race_allowed(quest.and_then(|quest| quest.required_races), race_id) && class_allowed(quest.and_then(|quest| quest.required_classes), class_id)
}

fn audience(player: &PlayerContext) -> Audience {
    (player.race_id, player.class_id)
}

/// A candidate that can still end up alive (at most `max_contradictions` views lacked it) or, if
/// it has variants, in an OR group (at most `max_contradictions` views that could take it lacked
/// it). Both only grow, so a candidate that can do neither is dropped for good.
struct Tracked {
    /// The views (by index) that had the target at every sighting, ascending.
    survived_in: Vec<u32>,
    /// For targets with variants: the views that could take the target (race/class) but lacked it.
    lacking: Option<u32>,
    fresh: u32,
    immediate: u32,
    latest: u32,
    evidence: Vec<Evidence>,
}

/// One quest's views so far, in character order.
pub struct QuestFold {
    quest_id: QuestId,
    views: u32,
    /// The givers the quest was first seen at.
    seen_givers: FxHashSet<GiverRef>,
    min_level: Option<i64>,
    /// The level of each view that first saw the quest within `fresh_seconds` of a level-up.
    level_ups: Vec<Option<i64>>,
    /// The last complete lists from the giver that lacked some survivors.
    last_lists: Vec<EarlierVisit>,
    /// The views (by index) that reached the giver before first seeing the quest there.
    arrivals: Vec<(u32, EarlierVisit)>,
    /// How many views each audience had, to count the earlier views that could take a target.
    audiences: BTreeMap<Audience, u32>,
    /// In the order targets were first met: OR groups resolve ties in that order.
    tracked: IndexMap<QuestId, Tracked, FxBuildHasher>,
    /// Targets with variants met but not tracked, so meeting them again stays cheap.
    untracked: FxHashSet<QuestId>,
}

impl QuestFold {
    pub fn new(quest_id: QuestId) -> Self {
        QuestFold {
            quest_id,
            views: 0,
            seen_givers: FxHashSet::default(),
            min_level: None,
            level_ups: Vec::new(),
            last_lists: Vec::new(),
            arrivals: Vec::new(),
            audiences: BTreeMap::new(),
            tracked: IndexMap::default(),
            untracked: FxHashSet::default(),
        }
    }

    /// Adds the next character's view of the quest.
    pub fn add(&mut self, view: &CharacterView, catalog: &QuestCatalog, params: &Params, variants: &VariantIndex) {
        let index = self.views;
        self.views += 1;
        let max_contradictions = params.max_contradictions;

        self.seen_givers.extend(view.giver.filter(|giver| giver.kind != GiverKind::Item));
        if let Some(level) = view.level {
            self.min_level = Some(self.min_level.map_or(level, |min| min.min(level)));
        }
        if view.level_up_gap.is_some_and(|gap| gap <= params.fresh_seconds) {
            self.level_ups.push(view.level);
        }
        if let Some(list) = view.last_list.as_ref().filter(|list| !list.not_completed.is_empty()) {
            self.last_lists.push(list.clone());
        }
        if let Some(arrival) = &view.arrival {
            self.arrivals.push((index, arrival.clone()));
        }

        let latest_gap = view.completion_gaps.values().copied().fold(f64::INFINITY, f64::min);
        for &target in &view.survivors {
            if !self.tracked.contains_key(&target) {
                if self.untracked.contains(&target) {
                    continue;
                }
                // Met for the first time, so every earlier view lacked it. (A target without
                // variants dropped earlier is met again only after more than max_contradictions
                // views, so it fails here again.)
                let alive_possible = index <= max_contradictions;
                let lacking = variants.has_variants(target).then(|| {
                    self.audiences
                        .iter()
                        .filter(|&(&audience, _)| can_take(catalog, target, audience))
                        .map(|(_, &views)| views)
                        .sum::<u32>()
                });
                let partial_possible = lacking.is_some_and(|lacking| lacking <= max_contradictions);
                if !alive_possible && !partial_possible {
                    if lacking.is_some() {
                        self.untracked.insert(target);
                    }
                    continue;
                }
                let tracked = Tracked {
                    survived_in: Vec::new(),
                    lacking,
                    fresh: 0,
                    immediate: 0,
                    latest: 0,
                    evidence: Vec::new(),
                };
                self.tracked.insert(target, tracked);
            }
            let entry = &mut self.tracked[&target];
            entry.survived_in.push(index);
            let gap = view.completion_gaps.get(&target).copied();
            if let Some(gap) = gap.filter(|&gap| gap <= params.fresh_seconds) {
                entry.fresh += 1;
                if gap <= params.immediate_seconds {
                    entry.immediate += 1;
                }
                if gap <= latest_gap + params.latest_tie_seconds {
                    entry.latest += 1;
                }
            }
            if let Some(gap) = gap.filter(|_| entry.evidence.len() < MAX_EVIDENCE) {
                entry.evidence.push(Evidence {
                    reference: view.episode_key.to_string(),
                    t: Some(view.t),
                    text: format!("{target} completed {}s before {}", to_fixed(gap, 1), describe_sighting(view)),
                });
            }
        }

        // The tracked targets this view lacked: one more contradiction, and for targets with
        // variants one more lacking view if the character could take them.
        let player = audience(view.player);
        let untracked = &mut self.untracked;
        self.tracked.retain(|&target, entry| {
            if entry.survived_in.last() == Some(&index) {
                return true;
            }
            if let Some(lacking) = entry.lacking.as_mut() {
                *lacking += u32::from(can_take(catalog, target, player));
            }
            let contradictions = index + 1 - entry.survived_in.len() as u32;
            let keep = contradictions <= max_contradictions || entry.lacking.is_some_and(|lacking| lacking <= max_contradictions);
            if !keep && entry.lacking.is_some() {
                untracked.insert(target);
            }
            keep
        });
        *self.audiences.entry(player).or_default() += 1;
    }

    /// The assessment, once every view is in.
    pub fn finish(self, catalog: &QuestCatalog, params: &Params, finishers: &FinisherIndex, variants: &VariantIndex) -> QuestAssessment {
        let quest_id = self.quest_id;
        let characters = self.views;
        let max_contradictions = params.max_contradictions;
        let level_gate = catalog.quest(quest_id).and_then(|quest| quest.required_level).or(self.min_level).unwrap_or(0);
        let level_unlocked = self.level_ups.iter().filter(|&&level| level == Some(level_gate)).count() as u32;
        let blocked_lists = self
            .last_lists
            .into_iter()
            .filter(|list| list.level.unwrap_or(0) >= level_gate)
            .map(|list| list.not_completed)
            .collect();

        // A window only says something when the level already allowed the quest at arrival:
        // otherwise the quest may simply have unlocked by levelling up.
        let mut in_window = vec![false; characters as usize];
        let mut pending: FxHashMap<QuestId, u32> = FxHashMap::default();
        for (view, arrival) in self.arrivals.iter().filter(|(_, arrival)| arrival.level.unwrap_or(0) >= level_gate) {
            in_window[*view as usize] = true;
            for &target in &arrival.not_completed {
                *pending.entry(target).or_default() += 1;
            }
        }

        let candidates: IndexMap<QuestId, Candidate, FxBuildHasher> = self
            .tracked
            .into_iter()
            .map(|(target, tracked)| {
                let support = tracked.survived_in.len() as u32;
                let pending = pending.get(&target).copied().unwrap_or(0);
                // A view's pending targets are among its survivors.
                let windows = tracked.survived_in.iter().filter(|&&view| in_window[view as usize]).count() as u32;
                let stats = CandidateStats {
                    target,
                    support,
                    contradict: characters - support,
                    fresh: tracked.fresh,
                    immediate: tracked.immediate,
                    latest: tracked.latest,
                    pending,
                    background: windows - pending,
                    link: false,
                    evidence: tracked.evidence,
                };
                let candidate = Candidate {
                    stats,
                    survived_in: tracked.survived_in,
                    lacking: tracked.lacking,
                };
                (target, candidate)
            })
            .collect();

        let mut givers = self.seen_givers;
        if let Some(starters) = catalog.quest(quest_id).map(|quest| &quest.starters) {
            givers.extend(starters.npcs.iter().map(|&id| GiverRef { kind: GiverKind::Npc, id }));
            givers.extend(starters.objects.iter().map(|&id| GiverRef { kind: GiverKind::Object, id }));
        }
        let is_linked = |target: QuestId| finishers.turned_in_at_any(target, &givers);
        let or_groups = or_groups(&candidates, characters, max_contradictions, &is_linked, variants);
        let mut alive: Vec<CandidateStats> = candidates
            .into_values()
            .map(|candidate| candidate.stats)
            .filter(|stats| stats.contradict <= max_contradictions)
            .map(|stats| CandidateStats {
                link: is_linked(stats.target),
                ..stats
            })
            .collect();
        alive.sort_by_key(|stats| stats.target);

        QuestAssessment {
            quest_id,
            characters,
            level_unlocked,
            blocked_lists,
            alive,
            or_groups,
        }
    }
}

/// A tracked target's counts, with what OR groups read besides them.
struct Candidate {
    stats: CandidateStats,
    survived_in: Vec<u32>,
    lacking: Option<u32>,
}

/// Faction-split prerequisites: the Alliance characters had P1, the Horde ones P2. Each member is
/// contradicted overall, but only by characters that could never take it (race/class), the
/// members are pairwise audience-exclusive same-name variants, and together they cover every
/// character.
fn or_groups(
    candidates: &IndexMap<QuestId, Candidate, FxBuildHasher>,
    characters: u32,
    max_contradictions: u32,
    is_linked: &dyn Fn(QuestId) -> bool,
    variants: &VariantIndex,
) -> Vec<OrGroup> {
    let is_partial = |target: QuestId| {
        candidates
            .get(&target)
            .is_some_and(|candidate| candidate.stats.contradict > max_contradictions && candidate.lacking.is_some_and(|lacking| lacking <= max_contradictions))
    };

    let mut groups: Vec<OrGroup> = Vec::new();
    for candidate in candidates.values() {
        let target = candidate.stats.target;
        let others = variants.variants_of(target);
        if others.is_empty() || !is_partial(target) {
            continue;
        }
        // Greedy by support keeps one copy per audience (a chain of same-name steps shares an
        // audience). Stable, so equal support keeps ascending ids.
        let mut others: Vec<QuestId> = others.into_iter().filter(|&other| is_partial(other)).collect();
        others.sort_by_key(|other| Reverse(candidates[other].stats.support));
        let mut members = vec![target];
        for other in others {
            if members.iter().all(|&member| variants.exclusive_audiences(member, other)) {
                members.push(other);
            }
        }
        members.sort_unstable();
        if members.len() < 2 || groups.iter().any(|group| group.members == members) {
            continue;
        }

        let mut supporting: Vec<u32> = members.iter().flat_map(|member| &candidates[member].survived_in).copied().collect();
        supporting.sort_unstable();
        supporting.dedup();
        let support = supporting.len() as u32;
        let contradict = characters - support;
        if contradict > max_contradictions {
            continue;
        }
        let mut summed = CandidateStats {
            support,
            contradict,
            ..CandidateStats::new(members[0])
        };
        for &member in &members {
            let own = &candidates[&member].stats;
            summed.fresh += own.fresh;
            summed.immediate += own.immediate;
            summed.latest += own.latest;
            summed.pending += own.pending;
            summed.background += own.background;
            summed.link |= is_linked(member);
            summed.evidence.extend(own.evidence.iter().take(2).cloned());
        }
        groups.push(OrGroup { members, stats: summed });
    }
    groups
}
