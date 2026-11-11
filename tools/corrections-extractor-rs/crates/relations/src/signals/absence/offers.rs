//! Pass 1 of the absence signal: everything known from moments a quest WAS available.
//!
//! A blocked moment (blocked.rs) only says "something stopped X here". What that something can
//! be is decided by contrast with every moment X was offered to anyone: a quest that sat in some
//! character's log while X was still offered cannot be what hides X while in the log, and so on.
//! This pass also learns which givers really start X (as observed, not as the catalog claims),
//! which quests are listed side by side, and which turn-ins hand off to which offers.

use super::params::AbsenceParams;
use super::states::{without_catch_ups, EpisodeStates, QuestSet};
use crate::model::{Faction, GiverRef, QuestEpisode, QuestEventKind, QuestId, SessionTime};
use indexmap::{IndexMap, IndexSet};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::collections::BTreeSet;
use std::rc::Rc;

/// A character, by its pseudonymous `QuestEpisode::character_key`.
pub type Character<'a> = &'a str;

/// What was observed while one quest was available.
#[derive(Default)]
pub struct QuestOffers<'a> {
    /// Characters the quest was offered to (in any list, or accepted).
    pub characters: FxHashSet<Character<'a>>,
    /// Lowest character level at an offer; blocked moments below it may be level gating.
    pub min_level: Option<i64>,
    pub factions: Vec<Faction>,
    pub races: FxHashSet<i64>,
    pub classes: FxHashSet<i64>,
    /// Listed as daily/weekly/repeatable: completion does not stick, so absence says little.
    pub periodic: bool,
    /// Other quest -> characters that had it in the log at some moment this quest was offered.
    pub in_log_when_offered: FxHashMap<QuestId, FxHashSet<Character<'a>>>,
    /// Other quest -> characters that had it completed at some moment this quest was offered.
    pub completed_when_offered: FxHashMap<QuestId, FxHashSet<Character<'a>>>,
    /// Quests completed at every giver offer of nearly every character (prerequisite suspects),
    /// in the order the offers first showed them. Filled by build(); see
    /// AbsenceParams::required_tolerance.
    pub required: QuestSet,
    /// Same-name quest groups (Forever's zone and race copies) of which one member was completed
    /// at nearly every offer, while no single member was: "any one of these" prerequisites.
    /// Smallest member -> members seen completed (ascending). Filled by build().
    pub required_any: IndexMap<QuestId, Vec<QuestId>, FxBuildHasher>,
    /// Per character, the earliest offer.
    pub first_offered_at: FxHashMap<Character<'a>, Moment>,
    /// Per character, the quests completed at each of their giver offers (ascending). The input
    /// of build(); characters in the order they were first offered the quest.
    completed_at_each_offer: IndexMap<Character<'a>, Vec<QuestId>, FxBuildHasher>,
}

#[derive(Default)]
pub struct OfferIndex<'a> {
    pub quests: IndexMap<QuestId, QuestOffers<'a>, FxBuildHasher>,
    /// Giver -> quests that giver was seen offering, in the order they were first seen.
    pub quests_by_giver: FxHashMap<GiverRef, QuestSet>,
    /// (a, b), a < b: listed together in one offer list at least once.
    pub co_offered: FxHashSet<(QuestId, QuestId)>,
    /// (x, y) -> givers where y was offered within the hand-off window after x was turned in there.
    pub handoffs: FxHashMap<(QuestId, QuestId), FxHashSet<GiverRef>>,
    /// Quest -> givers it was seen turned in at.
    pub finishers_of: FxHashMap<QuestId, FxHashSet<GiverRef>>,
    /// Quest -> its completion bundle (ascending, 2+ quests that always complete together).
    pub bundles: FxHashMap<QuestId, Vec<QuestId>>,
    pub episodes: usize,
}

impl OfferIndex<'_> {
    /// The quest standing in for its completion bundle (its smallest member), or the quest itself.
    pub fn representative(&self, quest_id: QuestId) -> QuestId {
        self.bundles.get(&quest_id).map_or(quest_id, |bundle| bundle[0])
    }

    /// A representative's bundle, or just the quest.
    pub fn bundle_of<'s>(&'s self, quest_id: &'s QuestId) -> &'s [QuestId] {
        self.bundles.get(quest_id).map_or(std::slice::from_ref(quest_id), Vec::as_slice)
    }

    pub fn listed_together(&self, a: QuestId, b: QuestId) -> bool {
        self.co_offered.contains(&pair(a, b))
    }

    /// Some giver offered `to` right after `from` was turned in there.
    pub fn hands_off(&self, from: QuestId, to: QuestId) -> bool {
        self.handoffs.contains_key(&(from, to))
    }

    /// `from` was turned in at a giver that offered `to` right after but never offered `from` itself.
    pub fn is_lead_in(&self, from: QuestId, to: QuestId) -> bool {
        self.handoffs.get(&(from, to)).is_some_and(|givers| {
            givers
                .iter()
                .any(|giver| !self.quests_by_giver.get(giver).is_some_and(|quests| quests.contains(&from)))
        })
    }
}

/// The unordered pair as a key: smaller id first.
pub fn pair(a: QuestId, b: QuestId) -> (QuestId, QuestId) {
    (a.min(b), a.max(b))
}

/// A point in one character's play history: which of their sessions, and when in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
    /// QuestEpisode::session_order: session names are missing for a third of sessions, so never
    /// order by them.
    pub session: u32,
    pub t: SessionTime,
}

impl Moment {
    pub fn of(episode: &QuestEpisode, t: SessionTime) -> Moment {
        Moment {
            session: episode.session_order,
            t,
        }
    }

    pub fn is_earlier(self, other: Moment) -> bool {
        self.session < other.session || (self.session == other.session && self.t < other.t)
    }
}

struct OfferMoment {
    t: SessionTime,
    /// Reached through a known giver. Giverless details are party shares and pushed quests.
    from_giver: bool,
}

pub struct OfferIndexBuilder<'a> {
    params: AbsenceParams,
    index: OfferIndex<'a>,
    /// Quest -> quests in every completed-set batch it arrived in, and how many batches that was.
    batch_mates: FxHashMap<QuestId, FxHashSet<QuestId>>,
    batch_counts: FxHashMap<QuestId, usize>,
}

impl<'a> OfferIndexBuilder<'a> {
    pub fn new(params: AbsenceParams) -> Self {
        OfferIndexBuilder {
            params,
            index: OfferIndex::default(),
            batch_mates: FxHashMap::default(),
            batch_counts: FxHashMap::default(),
        }
    }

    pub fn add(&mut self, episode: &'a QuestEpisode) {
        self.index.episodes += 1;
        let caught_up = without_catch_ups(episode, self.params.catch_up_batch);
        let mut states = EpisodeStates::new(&caught_up, self.params.state_slack);
        let mut moments: IndexMap<QuestId, Vec<OfferMoment>, FxBuildHasher> = IndexMap::default();

        for snapshot in &episode.offers {
            for offered in &snapshot.available {
                self.note(&mut moments, offered.id, snapshot.t, snapshot.giver);
                if offered.frequency.unwrap_or(1) > 1 || offered.repeatable == Some(true) {
                    self.index.quests.entry(offered.id).or_default().periodic = true;
                }
            }
            for (i, a) in snapshot.available.iter().enumerate() {
                for b in &snapshot.available[i + 1..] {
                    if a.id != b.id {
                        self.index.co_offered.insert(pair(a.id, b.id));
                    }
                }
            }
        }
        // A successful accept passed the server's availability check, however the dialog was reached.
        for event in &episode.quest_events {
            match (event.kind, event.giver) {
                (QuestEventKind::Accepted, giver) => self.note(&mut moments, event.quest_id, event.t, giver),
                (QuestEventKind::TurnedIn, Some(giver)) => {
                    self.index.finishers_of.entry(event.quest_id).or_default().insert(giver);
                }
                _ => {}
            }
        }

        for (quest_id, list) in moments {
            self.record_offers(episode, &mut states, quest_id, list);
        }
        self.record_handoffs(episode);
        self.record_batches(&caught_up);
    }

    fn note(&mut self, moments: &mut IndexMap<QuestId, Vec<OfferMoment>, FxBuildHasher>, quest_id: QuestId, t: SessionTime, giver: Option<GiverRef>) {
        moments.entry(quest_id).or_default().push(OfferMoment {
            t,
            from_giver: giver.is_some(),
        });
        if let Some(giver) = giver {
            self.index.quests_by_giver.entry(giver).or_default().insert(quest_id);
        }
    }

    /// `name_of` groups same-name copies for `required_any`.
    pub fn build<'c>(self, name_of: impl Fn(QuestId) -> Option<&'c str>) -> OfferIndex<'a> {
        let OfferIndexBuilder {
            params,
            mut index,
            batch_mates,
            batch_counts,
        } = self;
        for (&quest_id, mates) in &batch_mates {
            let mut bundle: Vec<QuestId> = mates
                .iter()
                .copied()
                .filter(|mate| {
                    batch_mates.get(mate).is_some_and(|theirs| theirs.contains(&quest_id))
                        && batch_counts.get(mate).is_some_and(|&count| count >= params.min_bundle_batches)
                })
                .collect();
            if (2..=params.max_bundle).contains(&bundle.len()) {
                bundle.sort_unstable();
                index.bundles.insert(quest_id, bundle);
            }
        }
        for (&quest_id, offers) in &mut index.quests {
            offers.find_prerequisites(quest_id, &name_of, params.required_tolerance);
        }
        index
    }

    fn record_offers(&mut self, episode: &'a QuestEpisode, states: &mut EpisodeStates, quest_id: QuestId, mut moments: Vec<OfferMoment>) {
        let offers = self.index.quests.entry(quest_id).or_default();
        let character = episode.character_key.as_str();
        offers.characters.insert(character);
        let player = &episode.player;
        if let Some(faction) = player.faction.filter(|faction| !offers.factions.contains(faction)) {
            offers.factions.push(faction);
        }
        offers.races.extend(player.race_id);
        offers.classes.extend(player.class_id);

        moments.sort_by(|a, b| a.t.total_cmp(&b.t));
        let first = Moment::of(episode, moments[0].t);
        if offers.first_offered_at.get(character).is_none_or(|&known| first.is_earlier(known)) {
            offers.first_offered_at.insert(character, first);
        }

        for moment in &moments {
            if let Some(level) = states.timeline.level_at(moment.t) {
                offers.min_level = Some(offers.min_level.map_or(level, |min| min.min(level)));
            }
            for &other in &states.log(moment.t) {
                if other != quest_id {
                    offers.in_log_when_offered.entry(other).or_default().insert(character);
                }
            }
        }

        // Completions only grow within a session (daily resets aside), so the first and last
        // offer bound the union and the intersection over every offer in between.
        if episode.completed.is_none() {
            return;
        }
        let at_first = states.completed(moments[0].t);
        let at_last = states.completed(moments[moments.len() - 1].t);
        let both = if Rc::ptr_eq(&at_first, &at_last) {
            vec![&at_last]
        } else {
            vec![&at_last, &at_first]
        };
        for &other in both.into_iter().flat_map(|set| set.iter()) {
            if other != quest_id {
                offers.completed_when_offered.entry(other).or_default().insert(character);
            }
        }

        // Prerequisites only from giver offers: shared and pushed quests skip the giver's checks.
        let mut from_giver = moments.iter().filter(|moment| moment.from_giver).map(|moment| moment.t);
        let Some(first_from_giver) = from_giver.next() else { return };
        let last_from_giver = from_giver.next_back().unwrap_or(first_from_giver);
        let at_first_giver = states.completed(first_from_giver);
        let at_last_giver = states.completed(last_from_giver);
        let mut completed: Vec<QuestId> = at_first_giver
            .iter()
            .copied()
            .filter(|&other| other != quest_id && at_last_giver.contains(&other))
            .collect();
        completed.sort_unstable();
        if let Some(earlier) = offers.completed_at_each_offer.get(character) {
            completed = intersect_sorted(earlier, &completed);
        }
        offers.completed_at_each_offer.insert(character, completed);
    }

    fn record_batches(&mut self, episode: &QuestEpisode) {
        for change in episode.completed.iter().flat_map(|completed| &completed.changes) {
            let batch = change.add.as_deref().unwrap_or_default();
            for &quest_id in batch {
                *self.batch_counts.entry(quest_id).or_default() += 1;
                self.batch_mates
                    .entry(quest_id)
                    .and_modify(|mates| mates.retain(|mate| batch.contains(mate)))
                    .or_insert_with(|| batch.iter().copied().collect());
            }
        }
    }

    /// "Turned in x at giver g, and g offered y within the window": y is handed off by x's turn-in.
    fn record_handoffs(&mut self, episode: &QuestEpisode) {
        let window = self.params.handoff_window;
        for turn_in in &episode.quest_events {
            let (QuestEventKind::TurnedIn, Some(giver)) = (turn_in.kind, turn_in.giver) else {
                continue;
            };
            let within = |t: SessionTime| t >= turn_in.t && t <= turn_in.t + window;
            let mut offered_after: FxHashSet<QuestId> = FxHashSet::default();
            for snapshot in episode.offers.iter().filter(|snapshot| within(snapshot.t) && snapshot.giver == Some(giver)) {
                offered_after.extend(snapshot.available.iter().map(|offered| offered.id));
            }
            for event in &episode.quest_events {
                if within(event.t) && matches!(event.kind, QuestEventKind::Detail | QuestEventKind::Accepted) && event.giver == Some(giver) {
                    offered_after.insert(event.quest_id);
                }
            }
            offered_after.remove(&turn_in.quest_id);
            for next in offered_after {
                self.index.handoffs.entry((turn_in.quest_id, next)).or_default().insert(giver);
            }
        }
    }
}

impl QuestOffers<'_> {
    /// Fills `required` and `required_any` from `completed_at_each_offer`.
    fn find_prerequisites<'c>(&mut self, quest_id: QuestId, name_of: impl Fn(QuestId) -> Option<&'c str>, tolerance: f64) {
        let characters = self.completed_at_each_offer.len();
        let enough = characters - (characters as f64 * tolerance).floor() as usize;
        let own_name = name_of(quest_id);
        let mut counts: IndexMap<QuestId, usize, FxBuildHasher> = IndexMap::default();
        let mut group_counts: IndexMap<&str, usize, FxBuildHasher> = IndexMap::default();
        let mut group_members: FxHashMap<&str, BTreeSet<QuestId>> = FxHashMap::default();
        for completed in self.completed_at_each_offer.values() {
            let mut groups_here: IndexSet<&str, FxBuildHasher> = IndexSet::default();
            for &other in completed {
                *counts.entry(other).or_default() += 1;
                let Some(name) = name_of(other).filter(|&name| Some(name) != own_name) else {
                    continue;
                };
                groups_here.insert(name);
                group_members.entry(name).or_default().insert(other);
            }
            for name in groups_here {
                *group_counts.entry(name).or_default() += 1;
            }
        }
        self.required = counts.into_iter().filter(|&(_, count)| count >= enough).map(|(other, _)| other).collect();
        for (name, count) in group_counts {
            let members: Vec<QuestId> = group_members[name].iter().copied().collect();
            if members.len() < 2 || count < enough || members.iter().any(|member| self.required.contains(member)) {
                continue;
            }
            self.required_any.insert(members[0], members);
        }
    }
}

fn intersect_sorted(a: &[QuestId], b: &[QuestId]) -> Vec<QuestId> {
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    result
}
