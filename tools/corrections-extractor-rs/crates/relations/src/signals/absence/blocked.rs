//! Pass 2 of the absence signal: moments a giver's complete list lacked a quest it starts, while
//! the character could otherwise have taken it, and what could explain each one ([`BlockKind`]).
//!
//! Completed and missing suspects stand for their whole completion bundle (offers.rs), keyed by
//! its representative: zone copies that complete together are one cause, not several.
//! Everything consistent with all offer moments (offers.rs) is a suspect. A character offered X a
//! minute earlier usually leaves one suspect (what changed in between), a character with no offer
//! of their own leaves hundreds. Suspects get a prior weight (locality.rs: unrelated quests count
//! little); attribute.rs then decides who takes the blame across all moments of X.

use super::locality::Locality;
use super::offers::{Character, Moment, OfferIndex, QuestOffers};
use super::params::AbsenceParams;
use super::states::{EpisodeStates, QuestSet};
use crate::eligibility::{has_untracked_requirements, is_statically_eligible};
use crate::model::{CatalogQuest, GiverRef, OfferSource, QuestCatalog, QuestEpisode, QuestEventKind, QuestId, SessionTime};
use crate::text::to_fixed;
use crate::timeline::Completion;
use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::rc::Rc;

/// How a suspect Y (or prerequisite P) explains "X missing from the list at t".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// Y was in the log at t, and Y was never in the log while X was offered.
    InLog,
    /// Y was completed at t, and Y was never completed while X was offered.
    Completed,
    /// P was never taken by t, but P was completed at every offer of X.
    Missing,
    /// The same, but P was in the log at t. A breadcrumb that nearly everyone does looks exactly
    /// like this, so only Missing proves a prerequisite.
    Pending,
    /// No member of same-name group G was completed at t, but one was at every offer. Keyed by the
    /// smallest member of a QuestOffers::required_any group.
    MissingAny,
}

/// A cause suspected for blocked moments: `kind` of `quest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Explanation {
    pub kind: BlockKind,
    pub quest: QuestId,
}

pub struct Suspect {
    pub explanation: Explanation,
    /// Prior weight: 1 for a structurally related quest, AbsenceParams::unrelated_weight otherwise.
    pub weight: f64,
    /// Reviewable state description, e.g. "92742 in log (accepted at 886.1s)".
    pub detail: String,
}

pub struct BlockedMoment<'a> {
    pub character: Character<'a>,
    /// QuestEpisode::key
    pub reference: &'a str,
    pub t: SessionTime,
    /// "npc:123 list at 45.0s lacked 678"
    pub what: String,
    /// The character was offered X before this moment, so its untracked requirements were met.
    pub transition: bool,
    pub suspects: Vec<Suspect>,
    /// Total prior weight of the suspects too unlikely to keep individually.
    pub residual_weight: f64,
}

#[derive(Default)]
pub struct BlockedQuest<'a> {
    /// Characters with at least one blocked moment that passed gating.
    pub characters: FxHashSet<Character<'a>>,
    /// Characters with a blocked moment nothing consistent explains (an unmodelled gate).
    pub unexplained: FxHashSet<Character<'a>>,
    pub moments: Vec<BlockedMoment<'a>>,
}

/// Why a candidate blocked moment was set aside, for the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SkipReason {
    NoCompletedHistory,
    NoQuestLog,
    GossipSubMenu,
    Uncataloged,
    Periodic,
    Seasonal,
    UnknownLevel,
    CatalogRaceClassLevel,
    BelowOfferedLevel,
    UnofferedFaction,
    UnofferedRace,
    UnofferedClass,
    UntrackedRequirements,
    /// The quest itself was completed, in the log or in a dialog around the read.
    OwnState,
}

impl SkipReason {
    pub fn name(self) -> &'static str {
        match self {
            SkipReason::NoCompletedHistory => "noCompletedHistory",
            SkipReason::NoQuestLog => "noQuestLog",
            SkipReason::GossipSubMenu => "gossipSubMenu",
            SkipReason::Uncataloged => "uncataloged",
            SkipReason::Periodic => "periodic",
            SkipReason::Seasonal => "seasonal",
            SkipReason::UnknownLevel => "unknownLevel",
            SkipReason::CatalogRaceClassLevel => "catalogRaceClassLevel",
            SkipReason::BelowOfferedLevel => "belowOfferedLevel",
            SkipReason::UnofferedFaction => "unofferedFaction",
            SkipReason::UnofferedRace => "unofferedRace",
            SkipReason::UnofferedClass => "unofferedClass",
            SkipReason::UntrackedRequirements => "untrackedRequirements",
            SkipReason::OwnState => "self",
        }
    }
}

/// What pass 2 found.
#[derive(Default)]
pub struct Blocked<'a> {
    /// Per quest with a blocked moment that passed gating, in the order they were first blocked.
    pub quests: IndexMap<QuestId, BlockedQuest<'a>, FxBuildHasher>,
    /// Set-aside moments per reason, reasons in the order first met.
    pub skipped: IndexMap<SkipReason, usize, FxBuildHasher>,
    pub blocked_moments: usize,
}

/// Skill, reputation and spell requirements: quests with equal ones have the same untracked gate.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Requirements {
    skill: Option<(i64, i64)>,
    min_rep: Option<(i64, i64)>,
    max_rep: Option<(i64, i64)>,
    spell: Option<i64>,
}

impl Requirements {
    fn of(quest: &CatalogQuest) -> Self {
        Requirements {
            skill: quest.required_skill,
            min_rep: quest.required_min_rep,
            max_rep: quest.required_max_rep,
            spell: quest.required_spell,
        }
    }
}

pub struct BlockedCollector<'i, 'a> {
    found: Blocked<'a>,
    index: &'i OfferIndex<'i>,
    locality: &'i Locality<'i>,
    catalog: &'a QuestCatalog,
    params: AbsenceParams,
    quests_by_requirements: FxHashMap<Requirements, Vec<QuestId>>,
    first_offer_by_requirements: FxHashMap<(Character<'a>, Requirements), Option<Moment>>,
}

impl<'i, 'a> BlockedCollector<'i, 'a> {
    pub fn new(index: &'i OfferIndex<'i>, locality: &'i Locality<'i>, catalog: &'a QuestCatalog, params: AbsenceParams) -> Self {
        let mut quests_by_requirements: FxHashMap<Requirements, Vec<QuestId>> = FxHashMap::default();
        for &quest_id in index.quests.keys() {
            if let Some(quest) = catalog.quest(quest_id).filter(|quest| has_untracked_requirements(quest)) {
                quests_by_requirements.entry(Requirements::of(quest)).or_default().push(quest_id);
            }
        }
        BlockedCollector {
            found: Blocked::default(),
            index,
            locality,
            catalog,
            params,
            quests_by_requirements,
            first_offer_by_requirements: FxHashMap::default(),
        }
    }

    pub fn finish(self) -> Blocked<'a> {
        self.found
    }

    pub fn add(&mut self, episode: &'a QuestEpisode) {
        if episode.completed.is_none() {
            return self.skip(SkipReason::NoCompletedHistory);
        }
        if episode.quest_log.is_empty() {
            return self.skip(SkipReason::NoQuestLog);
        }
        let mut states = EpisodeStates::new(episode, &self.params);
        let offered_here = offer_times_of(episode);
        let mut last_state: FxHashMap<QuestId, (Rc<QuestSet>, Vec<QuestId>)> = FxHashMap::default();
        let mut last_list_with_quests: FxHashMap<GiverRef, SessionTime> = FxHashMap::default();
        let index = self.index;

        for snapshot in &episode.offers {
            let Some(giver) = snapshot.giver else { continue };
            if !snapshot.list_complete || !self.params.blocked_sources.contains(&snapshot.source) {
                continue;
            }
            let listed: FxHashSet<QuestId> = snapshot
                .available
                .iter()
                .map(|offered| offered.id)
                .chain(snapshot.active.iter().flatten().copied())
                .collect();
            if !listed.is_empty() {
                last_list_with_quests.insert(giver, snapshot.t);
            } else if snapshot.source == OfferSource::Gossip {
                let previous = last_list_with_quests.get(&giver).copied();
                if previous.is_some_and(|previous| snapshot.t - previous <= self.params.sub_menu_window && !states.any_event_within(previous, snapshot.t)) {
                    self.skip(SkipReason::GossipSubMenu);
                    continue;
                }
            }
            // The character's state at this read, the same for every quest the list lacks.
            let mut at_read: Option<(Rc<QuestSet>, QuestSet, Vec<QuestId>)> = None;
            for &quest_id in index.quests_by_giver.get(&giver).into_iter().flatten() {
                if listed.contains(&quest_id) {
                    continue;
                }
                let offers = &index.quests[&quest_id];
                if let Some(reason) = self.gate(episode, &mut states, quest_id, offers, snapshot.t) {
                    self.skip(reason);
                    continue;
                }
                let (completed, log, log_key) = at_read.get_or_insert_with(|| {
                    let log = states.log(snapshot.t);
                    let mut log_key: Vec<QuestId> = log.iter().copied().collect();
                    log_key.sort_unstable();
                    (states.completed(snapshot.t), log, log_key)
                });
                // Repeated reads of one giver with nothing changed in between are one observation.
                if last_state
                    .get(&quest_id)
                    .is_some_and(|(last_completed, last_log)| Rc::ptr_eq(last_completed, completed) && last_log == log_key)
                {
                    continue;
                }
                last_state.insert(quest_id, (Rc::clone(completed), log_key.clone()));

                self.found.blocked_moments += 1;
                let read = ListRead {
                    t: snapshot.t,
                    giver,
                    completed,
                    log,
                    offered_here: offered_here.get(&quest_id).map_or(&[], Vec::as_slice),
                };
                self.record(episode, &states, quest_id, offers, read);
            }
        }
    }

    /// Why the character might not have been able to take the quest anyway; None if eligible.
    fn gate(&mut self, episode: &'a QuestEpisode, states: &mut EpisodeStates, quest_id: QuestId, offers: &QuestOffers, t: SessionTime) -> Option<SkipReason> {
        let Some(quest) = self.catalog.quest(quest_id) else {
            return Some(SkipReason::Uncataloged);
        };
        if offers.periodic || quest.is_repeatable() {
            return Some(SkipReason::Periodic);
        }
        if quest.needs_event() {
            return Some(SkipReason::Seasonal);
        }

        let player = &episode.player;
        let Some(level) = states.timeline.level_at(t) else {
            return Some(SkipReason::UnknownLevel);
        };
        if !is_statically_eligible(quest, player, Some(level), 0) {
            return Some(SkipReason::CatalogRaceClassLevel);
        }
        // Catalog levels can be off; only trust levels at which someone was actually offered the
        // quest. With no level known at any offer, no level is trusted.
        if offers.min_level.is_none_or(|min_level| level < min_level) {
            return Some(SkipReason::BelowOfferedLevel);
        }
        // Faction variants are never offered to the other faction; that is not exclusivity.
        if !player.faction.is_some_and(|faction| offers.factions.contains(&faction)) {
            return Some(SkipReason::UnofferedFaction);
        }
        let open = self.params.open_demographics;
        if offers.races.len() < open && !player.race_id.is_some_and(|race| offers.races.contains(&race)) {
            return Some(SkipReason::UnofferedRace);
        }
        if offers.classes.len() < open && !player.class_id.is_some_and(|class| offers.classes.contains(&class)) {
            return Some(SkipReason::UnofferedClass);
        }
        // Skill, reputation and spell requirements are not in the trace. An earlier offer to this
        // character of a quest with the very same requirements (often a zone copy of this quest)
        // shows they met them; skills and reputation rarely go back down.
        if has_untracked_requirements(quest) {
            let first = self.first_offer_with_requirements(Requirements::of(quest), &episode.character_key);
            if !first.is_some_and(|first| first.is_earlier(Moment::of(episode, t))) {
                return Some(SkipReason::UntrackedRequirements);
            }
        }

        let slack = self.params.self_slack;
        if states.is_completed(quest_id, t + slack) {
            return Some(SkipReason::OwnState);
        }
        if states.in_log(quest_id, t) || states.timeline.is_in_log_at(quest_id, t - slack) || states.timeline.is_in_log_at(quest_id, t + slack) {
            return Some(SkipReason::OwnState);
        }
        if states.touched(quest_id, t - slack, t + slack) {
            return Some(SkipReason::OwnState);
        }
        None
    }

    /// The character's earliest offer of any quest with these untracked requirements.
    fn first_offer_with_requirements(&mut self, requirements: Requirements, character: Character<'a>) -> Option<Moment> {
        let index = self.index;
        let quests = &self.quests_by_requirements;
        *self.first_offer_by_requirements.entry((character, requirements)).or_insert_with(|| {
            quests
                .get(&requirements)
                .into_iter()
                .flatten()
                .filter_map(|quest_id| index.quests.get(quest_id)?.first_offered_at.get(character).copied())
                .fold(None, |first, offered| match first {
                    Some(first) if !offered.is_earlier(first) => Some(first),
                    _ => Some(offered),
                })
        })
    }

    fn record(&mut self, episode: &'a QuestEpisode, states: &EpisodeStates, quest_id: QuestId, offers: &QuestOffers, read: ListRead) {
        let index = self.index;
        let t = read.t;
        let suspects = suspects_at(index, offers, quest_id, &read, self.params.max_contradictions);
        let character = episode.character_key.as_str();
        if suspects.is_empty() {
            let blocked = self.found.quests.entry(quest_id).or_default();
            blocked.characters.insert(character);
            blocked.unexplained.insert(character);
            return;
        }
        let weights: Vec<f64> = suspects
            .iter()
            .map(|explanation| {
                let related = members_of(index, offers, explanation)
                    .iter()
                    .any(|&member| self.locality.reason(quest_id, member).is_some());
                if related {
                    1.0
                } else {
                    self.params.unrelated_weight
                }
            })
            .collect();
        // Folds, here and in attribute.rs and score.rs, add in the TypeScript reduce order, so the sums
        // (and the min_share and support thresholds on them) match it to the last bit.
        let total_weight: f64 = weights.iter().fold(0.0, |sum, weight| sum + weight);
        let mut kept: Vec<Suspect> = Vec::new();
        let mut residual_weight = 0.0;
        for (explanation, &weight) in suspects.iter().zip(&weights) {
            if weight / total_weight < self.params.min_share {
                residual_weight += weight;
            } else {
                let detail = describe_state(states, explanation.kind, members_of(index, offers, explanation), t);
                kept.push(Suspect {
                    explanation: *explanation,
                    weight,
                    detail,
                });
            }
        }

        let earlier_offer = read.offered_here.iter().rev().find(|&&offered_at| offered_at < t);
        let mut what = format!("{} list at {}s lacked {quest_id}", giver_label(read.giver), to_fixed(t, 1));
        if let Some(&offered_at) = earlier_offer {
            what += &format!(" (offered at {}s)", to_fixed(offered_at, 1));
        }
        let moment = BlockedMoment {
            character,
            reference: &episode.key,
            t,
            what,
            transition: offers
                .first_offered_at
                .get(character)
                .is_some_and(|first| first.is_earlier(Moment::of(episode, t))),
            suspects: kept,
            residual_weight,
        };
        let blocked = self.found.quests.entry(quest_id).or_default();
        blocked.characters.insert(character);
        blocked.moments.push(moment);
    }

    fn skip(&mut self, reason: SkipReason) {
        *self.found.skipped.entry(reason).or_default() += 1;
    }
}

/// Every explanation of the quest missing from this read that no offer moment contradicts, in the
/// order: log, completed set, required quests, required groups.
fn suspects_at(index: &OfferIndex, offers: &QuestOffers, quest_id: QuestId, read: &ListRead, max_contradictions: usize) -> Vec<Explanation> {
    let ListRead { completed, log, .. } = *read;
    let consistent = |contradicting: Option<&u32>| contradicting.map_or(0, |&characters| characters as usize) <= max_contradictions;
    let required = &offers.required;
    let in_required_group = |other: QuestId| offers.required_any.values().any(|members| members.contains(&other));
    let mut suspects = Vec::new();
    let mut suspect = |kind: BlockKind, quest: QuestId| suspects.push(Explanation { kind, quest });
    // A required quest in the log is simply not completed yet: Pending covers it.
    for &other in log {
        if other == quest_id || required.contains(&other) || in_required_group(other) {
            continue;
        }
        if consistent(offers.in_log_when_offered.get(&other)) {
            suspect(BlockKind::InLog, other);
        }
    }
    let mut completed_bundles: FxHashSet<QuestId> = FxHashSet::default();
    for &other in completed {
        let representative = index.representative(other);
        if other == quest_id || !completed_bundles.insert(representative) {
            continue;
        }
        if consistent(offers.completed_when_offered.get(&representative)) {
            suspect(BlockKind::Completed, representative);
        }
    }
    let mut missing_bundles: FxHashSet<QuestId> = FxHashSet::default();
    for &other in required {
        let representative = index.representative(other);
        if completed.contains(&other) || !missing_bundles.insert(representative) {
            continue;
        }
        let pending = index.bundle_of(&representative).iter().any(|member| log.contains(member));
        suspect(if pending { BlockKind::Pending } else { BlockKind::Missing }, representative);
    }
    for (&representative, members) in &offers.required_any {
        if !members.iter().any(|member| completed.contains(member)) {
            suspect(BlockKind::MissingAny, representative);
        }
    }
    suspects
}

/// The quests a suspect stands for: a required_any group, a quest in the log, or a completion bundle.
fn members_of<'s>(index: &'s OfferIndex, offers: &'s QuestOffers, explanation: &'s Explanation) -> &'s [QuestId] {
    match explanation.kind {
        BlockKind::MissingAny => &offers.required_any[&explanation.quest],
        BlockKind::InLog => std::slice::from_ref(&explanation.quest),
        BlockKind::Completed | BlockKind::Missing | BlockKind::Pending => index.bundle_of(&explanation.quest),
    }
}

/// One read of a giver's complete list, as seen for one quest missing from it.
struct ListRead<'r> {
    t: SessionTime,
    giver: GiverRef,
    completed: &'r QuestSet,
    log: &'r QuestSet,
    /// Ascending times the quest was listed or accepted in this episode.
    offered_here: &'r [SessionTime],
}

/// "npc:123", as the TypeScript absence signal keys givers (not [`GiverRef::label`]'s "npc 123").
fn giver_label(giver: GiverRef) -> String {
    format!("{}:{}", giver.kind.name(), giver.id)
}

fn describe_state(states: &EpisodeStates, kind: BlockKind, members: &[QuestId], t: SessionTime) -> String {
    let quest_id = members[0];
    let ids = members.iter().map(QuestId::to_string).collect::<Vec<_>>().join("/");
    match kind {
        BlockKind::Missing => format!("{ids} not taken"),
        BlockKind::Pending => format!("{ids} in log, not completed"),
        BlockKind::MissingAny => format!("none of {ids} completed"),
        BlockKind::InLog => {
            let accepted = states
                .episode
                .quest_events
                .iter()
                .rfind(|event| event.quest_id == quest_id && event.t <= t && event.kind == QuestEventKind::Accepted);
            match accepted {
                Some(accepted) => format!("{quest_id} in log (accepted at {}s)", to_fixed(accepted.t, 1)),
                None => format!("{quest_id} in log"),
            }
        }
        BlockKind::Completed => match states.timeline.completion_time(quest_id) {
            Some(Completion::At(completed_at)) => format!("{ids} completed at {}s", to_fixed(completed_at, 1)),
            Some(Completion::Initial) | None => format!("{ids} completed before the session"),
        },
    }
}

/// Per quest, the ascending times it was listed or accepted in this episode.
fn offer_times_of(episode: &QuestEpisode) -> FxHashMap<QuestId, Vec<SessionTime>> {
    let mut result: FxHashMap<QuestId, Vec<SessionTime>> = FxHashMap::default();
    for snapshot in &episode.offers {
        for offered in &snapshot.available {
            result.entry(offered.id).or_default().push(snapshot.t);
        }
    }
    for event in episode.quest_events.iter().filter(|event| event.kind == QuestEventKind::Accepted) {
        result.entry(event.quest_id).or_default().push(event.t);
    }
    for times in result.values_mut() {
        times.sort_by(f64::total_cmp);
    }
    result
}
