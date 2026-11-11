//! Folds every character's view of quest X into per-candidate counts. A candidate P stays
//! "alive" while (almost) every character had it completed at every sighting of X; the other
//! counts measure how specific P is to X, because quests everyone finishes early survive too.

use super::observe::CharacterView;
use super::variants::VariantIndex;
use super::{to_fixed, Params};
use crate::eligibility::{class_allowed, race_allowed};
use crate::model::{Evidence, GiverKind, GiverRef, QuestCatalog, QuestEpisode, QuestEventKind, QuestId};
use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::cmp::Reverse;

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

/// The quest's catalog starters and every giver it was first seen at.
fn quest_givers(views: &[CharacterView], catalog: &QuestCatalog, quest_id: QuestId) -> FxHashSet<GiverRef> {
    let starters = catalog.quest(quest_id).map(|quest| &quest.starters);
    let npcs = starters
        .into_iter()
        .flat_map(|starters| &starters.npcs)
        .map(|&id| GiverRef { kind: GiverKind::Npc, id });
    let objects = starters
        .into_iter()
        .flat_map(|starters| &starters.objects)
        .map(|&id| GiverRef { kind: GiverKind::Object, id });
    let seen = views.iter().filter_map(|view| view.giver).filter(|giver| giver.kind != GiverKind::Item);
    npcs.chain(objects).chain(seen).collect()
}

fn giver_kind_name(kind: GiverKind) -> &'static str {
    match kind {
        GiverKind::Npc => "npc",
        GiverKind::Object => "object",
        GiverKind::Item => "item",
    }
}

fn describe_sighting(view: &CharacterView) -> String {
    let at = view
        .giver
        .map(|giver| format!(" at {} {}", giver_kind_name(giver.kind), giver.id))
        .unwrap_or_default();
    let level = view.level.map_or_else(|| "?".to_string(), |level| level.to_string());
    format!("{} first seen ({}){at}, level {level}", view.quest_id, view.kind.name())
}

/// Aggregates the views of one quest (one view per character, in character order).
pub fn aggregate_quest(
    quest_id: QuestId,
    views: &[CharacterView],
    catalog: &QuestCatalog,
    params: &Params,
    finishers: &FinisherIndex,
    variants: &VariantIndex,
) -> QuestAssessment {
    let givers = quest_givers(views, catalog, quest_id);
    let min_level = views.iter().filter_map(|view| view.level).min();
    let level_gate = catalog.quest(quest_id).and_then(|quest| quest.required_level).or(min_level).unwrap_or(0);

    // In the order targets were first met: or_groups resolves ties in that order.
    let mut stats: IndexMap<QuestId, CandidateStats, FxBuildHasher> = IndexMap::default();
    let mut level_unlocked = 0;
    let mut blocked_lists = Vec::new();
    for view in views {
        if let Some(list) = view
            .last_list
            .as_ref()
            .filter(|list| list.level.unwrap_or(0) >= level_gate && !list.not_completed.is_empty())
        {
            blocked_lists.push(list.not_completed.clone());
        }
        if view.level_up_gap.is_some_and(|gap| gap <= params.fresh_seconds) && view.level == Some(level_gate) {
            level_unlocked += 1;
        }
        // A window only says something when the level already allowed the quest at arrival:
        // otherwise the quest may simply have unlocked by levelling up.
        let window = view
            .arrival
            .as_ref()
            .filter(|arrival| arrival.level.unwrap_or(0) >= level_gate)
            .map(|arrival| &arrival.not_completed);
        let latest_gap = view.completion_gaps.values().copied().fold(f64::INFINITY, f64::min);

        for &target in &view.survivors {
            let entry = stats.entry(target).or_insert_with(|| CandidateStats::new(target));
            entry.support += 1;
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
            if let Some(window) = window {
                if window.binary_search(&target).is_ok() {
                    entry.pending += 1;
                } else {
                    entry.background += 1;
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
    }

    let characters = views.len() as u32;
    for entry in stats.values_mut() {
        entry.contradict = characters - entry.support;
    }
    let is_linked = |target: QuestId| finishers.turned_in_at_any(target, &givers);
    let or_groups = or_groups(views, &stats, catalog, params, &is_linked, variants);
    let mut alive: Vec<CandidateStats> = stats
        .into_values()
        .filter(|entry| entry.contradict <= params.max_contradictions)
        .map(|entry| CandidateStats {
            link: is_linked(entry.target),
            ..entry
        })
        .collect();
    alive.sort_by_key(|entry| entry.target);

    QuestAssessment {
        quest_id,
        characters,
        level_unlocked,
        blocked_lists,
        alive,
        or_groups,
    }
}

/// Faction-split prerequisites: the Alliance characters had P1, the Horde ones P2. Each member is
/// contradicted overall, but only by characters that could never take it (race/class), the
/// members are pairwise audience-exclusive same-name variants, and together they cover every
/// character.
fn or_groups(
    views: &[CharacterView],
    stats: &IndexMap<QuestId, CandidateStats, FxBuildHasher>,
    catalog: &QuestCatalog,
    params: &Params,
    is_linked: &dyn Fn(QuestId) -> bool,
    variants: &VariantIndex,
) -> Vec<OrGroup> {
    let can_take = |target: QuestId, view: &CharacterView| {
        let quest = catalog.quest(target);
        race_allowed(quest.and_then(|quest| quest.required_races), view.player.race_id)
            && class_allowed(quest.and_then(|quest| quest.required_classes), view.player.class_id)
    };
    let is_partial = |target: QuestId| {
        stats.get(&target).is_some_and(|entry| {
            entry.contradict > params.max_contradictions
                && views.iter().filter(|view| can_take(target, view) && !view.survived(target)).count() <= params.max_contradictions as usize
        })
    };

    let mut groups: Vec<OrGroup> = Vec::new();
    for entry in stats.values() {
        let others = variants.variants_of(entry.target);
        if others.is_empty() || !is_partial(entry.target) {
            continue;
        }
        // Greedy by support keeps one copy per audience (a chain of same-name steps shares an
        // audience). Stable, so equal support keeps ascending ids.
        let mut others: Vec<QuestId> = others.into_iter().filter(|&other| is_partial(other)).collect();
        others.sort_by_key(|other| Reverse(stats[other].support));
        let mut members = vec![entry.target];
        for other in others {
            if members.iter().all(|&member| variants.exclusive_audiences(member, other)) {
                members.push(other);
            }
        }
        members.sort_unstable();
        if members.len() < 2 || groups.iter().any(|group| group.members == members) {
            continue;
        }

        let support = views.iter().filter(|view| members.iter().any(|&member| view.survived(member))).count() as u32;
        let contradict = views.len() as u32 - support;
        if contradict > params.max_contradictions {
            continue;
        }
        let mut summed = CandidateStats {
            support,
            contradict,
            ..CandidateStats::new(members[0])
        };
        for &member in &members {
            let own = &stats[&member];
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
