//! Pass 1 of the absence signal: everything known from moments a quest WAS available.
//!
//! A blocked moment (blocked.rs) only says "something stopped X here". What that something can
//! be is decided by contrast with every moment X was offered to anyone: a quest that sat in some
//! character's log while X was still offered cannot be what hides X while in the log, and so on.
//! This pass also learns which givers really start X (as observed, not as the catalog claims),
//! which quests are listed side by side, and which turn-ins hand off to which offers.
//!
//! Later passes only ask how many characters saw a state while X was offered, so a character's
//! offers are tallied once all of their episodes are read (a character's episodes are together,
//! in one bucket): only the characters being read keep their states.

use super::for_each_character_chunk;
use super::params::AbsenceParams;
use super::states::{EpisodeStates, QuestSet};
use crate::episode_store::Episodes;
use crate::model::{Faction, GiverRef, QuestCatalog, QuestEpisode, QuestEventKind, QuestId, SessionTime};
use crate::signals::{CharacterId, CharacterIds};
use anyhow::Result;
use indexmap::IndexMap;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::rc::Rc;
use std::sync::Arc;

/// What was observed while one quest was available.
#[derive(Default)]
pub struct QuestOffers {
    /// Lowest character level at an offer; blocked moments below it may be level gating.
    pub min_level: Option<i64>,
    pub factions: Vec<Faction>,
    pub races: FxHashSet<i64>,
    pub classes: FxHashSet<i64>,
    /// Listed as daily/weekly/repeatable: completion does not stick, so absence says little.
    pub periodic: bool,
    /// Other quest -> characters that had it in the log at some moment this quest was offered.
    pub in_log_when_offered: FxHashMap<QuestId, u32>,
    /// Other quest -> characters that had it completed at some moment this quest was offered.
    pub completed_when_offered: FxHashMap<QuestId, u32>,
    /// Quests completed at every giver offer of nearly every character (prerequisite suspects),
    /// in the order the offers first showed them. See AbsenceParams::required_tolerance.
    pub required: QuestSet,
    /// Same-name quest groups (Forever's zone and race copies) of which one member was completed
    /// at nearly every offer, while no single member was: "any one of these" prerequisites.
    /// Smallest member -> members seen completed (ascending).
    pub required_any: IndexMap<QuestId, Vec<QuestId>, FxBuildHasher>,
    /// Per character the quest was offered to (in any list, or accepted), the earliest offer.
    pub first_offered_at: FxHashMap<CharacterId, Moment>,
    /// What `required` and `required_any` are made of once every character is tallied.
    prerequisites: PrerequisiteTally,
}

impl QuestOffers {
    /// Characters the quest was offered to.
    pub fn characters(&self) -> usize {
        self.first_offered_at.len()
    }

    /// Adds what the next character's episodes showed about the quest, `quest_id`. `shared_names`
    /// is for the prerequisite tally ([`PrerequisiteTally::add`]). The counts by other quest are
    /// only ever looked up, so the order their quests are added in does not matter.
    fn add(&mut self, quest_id: QuestId, character: CharacterId, seen: OfferedToCharacter, shared_names: &FxHashMap<QuestId, SharedName>) {
        self.periodic |= seen.periodic;
        if let Some(level) = seen.min_level {
            self.min_level = Some(self.min_level.map_or(level, |min| min.min(level)));
        }
        push_new(&mut self.factions, seen.factions);
        self.races.extend(seen.races);
        self.classes.extend(seen.classes);
        let first_offered_at = seen.first_offered_at.expect("every quest offered to a character has an offer moment");
        self.first_offered_at.insert(character, first_offered_at);
        for other in seen.in_log {
            *self.in_log_when_offered.entry(other).or_default() += 1;
        }
        for &other in seen.completed.iter().filter(|&&other| other != quest_id) {
            *self.completed_when_offered.entry(other).or_default() += 1;
        }
        if let Some(completed) = &seen.completed_at_each_giver_offer {
            let completed = completed.iter().copied().filter(|&other| other != quest_id);
            self.prerequisites.add(completed, shared_names.get(&quest_id).copied(), shared_names);
        }
    }
}

#[derive(Default)]
pub struct OfferIndex {
    pub quests: IndexMap<QuestId, QuestOffers, FxBuildHasher>,
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

impl OfferIndex {
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

/// Pass 1 over the Forever episodes, a bucket at a time. Characters are read side by side, each on
/// its own ([`CharacterOffers::read`]), and added to the index in the order the buckets list them:
/// insertion orders follow it. `characters` gives every character met its id, for the later
/// passes to use too.
pub fn index_offers(episodes: &dyn Episodes, catalog: &QuestCatalog, params: AbsenceParams, characters: &mut CharacterIds) -> Result<OfferIndex> {
    let mut builder = OfferIndexBuilder {
        params,
        shared_names: shared_names(catalog),
        index: OfferIndex::default(),
        batch_mates: FxHashMap::default(),
        batch_counts: FxHashMap::default(),
    };
    for_each_character_chunk(episodes, characters, |chunk| {
        let read: Vec<CharacterOffers> = chunk.par_iter().map(|character| CharacterOffers::read(&character.episodes, &params)).collect();
        builder.add(chunk.iter().map(|character| character.id).zip(read));
    })?;
    Ok(builder.build())
}

/// A quest name that several catalog quests have: Forever's zone and race copies.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SharedName(u32);

/// The names of catalog quests that share their name with another one. Only these can form a
/// same-name group of two or more.
fn shared_names(catalog: &QuestCatalog) -> FxHashMap<QuestId, SharedName> {
    let mut quests_named: FxHashMap<&str, usize> = FxHashMap::default();
    for name in catalog.quests.values().filter_map(|quest| quest.name.as_deref()) {
        *quests_named.entry(name).or_default() += 1;
    }
    let mut names: FxHashMap<&str, SharedName> = FxHashMap::default();
    catalog
        .quests
        .iter()
        .filter_map(|(&id, quest)| {
            let name = quest.name.as_deref().filter(|name| quests_named[name] >= 2)?;
            let next = SharedName(u32::try_from(names.len()).expect("fewer than 2^32 names"));
            Some((id, *names.entry(name).or_insert(next)))
        })
        .collect()
}

struct OfferMoment {
    t: SessionTime,
    /// Reached through a known giver. Giverless details are party shares and pushed quests.
    from_giver: bool,
}

/// What one character's episodes showed: their part of the index, before it is added to it.
#[derive(Default)]
struct CharacterOffers {
    episodes: usize,
    /// Per quest offered to the character, in the order the index first meets them in these
    /// episodes.
    quests: IndexMap<QuestId, OfferedToCharacter, FxBuildHasher>,
    quests_by_giver: FxHashMap<GiverRef, QuestSet>,
    co_offered: FxHashSet<(QuestId, QuestId)>,
    handoffs: FxHashMap<(QuestId, QuestId), FxHashSet<GiverRef>>,
    finishers_of: FxHashMap<QuestId, FxHashSet<GiverRef>>,
    /// The quests each completed-set change added (catch-up reads folded away), in order.
    completed_batches: Vec<Vec<QuestId>>,
}

/// What one character's episodes showed about one quest offered to them. Kept small: a chunk of
/// characters holds tens of thousands of these until they are added to the index.
#[derive(Default)]
struct OfferedToCharacter {
    periodic: bool,
    min_level: Option<i64>,
    /// Each once (a character rarely has more than one).
    factions: Vec<Faction>,
    races: Vec<i64>,
    classes: Vec<i64>,
    /// Set by the quest's first offer moment. A quest listed as periodic is met (and placed in the
    /// index's order of quests) while its list is read, just before its offer moments are, so it
    /// is None only in between.
    first_offered_at: Option<Moment>,
    /// Quests in the log at some offer, ascending.
    in_log: Vec<QuestId>,
    /// Quests completed at some offer, ascending. The quest itself may be among them: a
    /// character's quests share these lists ([`CompletedSets`]).
    completed: Arc<[QuestId]>,
    /// Quests completed at each of the character's giver offers, ascending; the quest itself may
    /// be among them.
    completed_at_each_giver_offer: Option<Arc<[QuestId]>>,
}

impl CharacterOffers {
    /// Reads one character's episodes, in session order.
    fn read(episodes: &[&QuestEpisode], params: &AbsenceParams) -> CharacterOffers {
        let mut reader = CharacterReader::default();
        for episode in episodes {
            reader.add(episode, params);
        }
        reader.finish()
    }
}

/// The completed sets a character had at offers. A character is offered dozens of quests at the
/// same few moments, so each quest names the sets it was offered at, and the quests offered at
/// the same ones share one list of what was completed.
#[derive(Default)]
struct CompletedSets {
    /// Each set's quests ascending; a set once per episode it was read in.
    sets: Vec<Arc<[QuestId]>>,
    /// Per quest offered, the sets at its offers and at its giver offers (indices into `sets`).
    of_quest: FxHashMap<QuestId, SetsOfQuest>,
}

#[derive(Default)]
struct SetsOfQuest {
    at_offers: Vec<usize>,
    /// None without a giver offer.
    at_giver_offers: Option<Vec<usize>>,
}

/// The sets an episode's states handed out so far, with their place in [`CompletedSets::sets`].
/// [`EpisodeStates::completed`] hands out the same set for reads with nothing completed in
/// between, and keeps each one for the episode.
type EpisodeSets = Vec<(Rc<QuestSet>, usize)>;

impl CompletedSets {
    /// `set`'s place in `sets`, added when this episode had not handed it out before.
    fn index_of(&mut self, episode_sets: &mut EpisodeSets, set: Rc<QuestSet>) -> usize {
        if let Some(&(_, index)) = episode_sets.iter().find(|(known, _)| Rc::ptr_eq(known, &set)) {
            return index;
        }
        let mut ascending: Vec<QuestId> = set.iter().copied().collect();
        ascending.sort_unstable();
        self.sets.push(ascending.into());
        episode_sets.push((set, self.sets.len() - 1));
        self.sets.len() - 1
    }

    /// Per quest, everything completed at some offer and everything completed at each giver
    /// offer. Quests with the same sets get the same lists.
    fn resolve(self, quests: &mut IndexMap<QuestId, OfferedToCharacter, FxBuildHasher>) {
        let CompletedSets { sets, of_quest } = self;
        let mut unions: FxHashMap<Vec<usize>, Arc<[QuestId]>> = FxHashMap::default();
        let mut intersections: FxHashMap<Vec<usize>, Arc<[QuestId]>> = FxHashMap::default();
        for (quest_id, mut of) in of_quest {
            let seen = &mut quests[&quest_id];
            of.at_offers.sort_unstable();
            seen.completed = Arc::clone(unions.entry(of.at_offers).or_insert_with_key(|indices| match indices.as_slice() {
                &[only] => Arc::clone(&sets[only]),
                indices => {
                    let mut union: Vec<QuestId> = indices.iter().flat_map(|&index| sets[index].iter().copied()).collect();
                    union.sort_unstable();
                    union.dedup();
                    union.into()
                }
            }));
            if let Some(mut at_giver_offers) = of.at_giver_offers {
                at_giver_offers.sort_unstable();
                let intersection = intersections.entry(at_giver_offers).or_insert_with_key(|indices| {
                    let (&first, rest) = indices.split_first().expect("each giver offer adds the sets it was read at");
                    rest.iter()
                        .fold(Arc::clone(&sets[first]), |all, &index| intersect_sorted(&all, &sets[index]).into())
                });
                seen.completed_at_each_giver_offer = Some(Arc::clone(intersection));
            }
        }
    }
}

/// Reads one character's episodes into [`CharacterOffers`].
#[derive(Default)]
struct CharacterReader {
    offers: CharacterOffers,
    completed: CompletedSets,
}

impl CharacterReader {
    fn finish(mut self) -> CharacterOffers {
        self.completed.resolve(&mut self.offers.quests);
        self.offers
    }

    fn add(&mut self, episode: &QuestEpisode, params: &AbsenceParams) {
        self.offers.episodes += 1;
        let mut states = EpisodeStates::new(episode, params);
        let mut episode_sets = EpisodeSets::new();
        let mut moments: IndexMap<QuestId, Vec<OfferMoment>, FxBuildHasher> = IndexMap::default();

        for snapshot in &episode.offers {
            for offered in &snapshot.available {
                self.note(&mut moments, offered.id, snapshot.t, snapshot.giver);
                if offered.frequency.unwrap_or(1) > 1 || offered.repeatable == Some(true) {
                    self.offers.quests.entry(offered.id).or_default().periodic = true;
                }
            }
            for (i, a) in snapshot.available.iter().enumerate() {
                for b in &snapshot.available[i + 1..] {
                    if a.id != b.id {
                        self.offers.co_offered.insert(pair(a.id, b.id));
                    }
                }
            }
        }
        // A successful accept passed the server's availability check, however the dialog was reached.
        for event in &episode.quest_events {
            match (event.kind, event.giver) {
                (QuestEventKind::Accepted, giver) => self.note(&mut moments, event.quest_id, event.t, giver),
                (QuestEventKind::TurnedIn, Some(giver)) => {
                    self.offers.finishers_of.entry(event.quest_id).or_default().insert(giver);
                }
                _ => {}
            }
        }

        for (quest_id, list) in moments {
            self.record_offers(episode, &mut states, &mut episode_sets, quest_id, list);
        }
        self.offers.record_handoffs(episode, params.handoff_window);
        if let Some(completed) = &states.completed_history {
            self.offers
                .completed_batches
                .extend(completed.changes.iter().filter_map(|change| change.add.clone()));
        }
    }

    fn note(&mut self, moments: &mut IndexMap<QuestId, Vec<OfferMoment>, FxBuildHasher>, quest_id: QuestId, t: SessionTime, giver: Option<GiverRef>) {
        moments.entry(quest_id).or_default().push(OfferMoment {
            t,
            from_giver: giver.is_some(),
        });
        if let Some(giver) = giver {
            self.offers.quests_by_giver.entry(giver).or_default().insert(quest_id);
        }
    }

    fn record_offers(
        &mut self,
        episode: &QuestEpisode,
        states: &mut EpisodeStates,
        episode_sets: &mut EpisodeSets,
        quest_id: QuestId,
        mut moments: Vec<OfferMoment>,
    ) {
        let seen = self.offers.quests.entry(quest_id).or_default();
        let player = &episode.player;
        push_new(&mut seen.factions, player.faction);
        push_new(&mut seen.races, player.race_id);
        push_new(&mut seen.classes, player.class_id);

        moments.sort_by(|a, b| a.t.total_cmp(&b.t));
        let first = Moment::of(episode, moments[0].t);
        if seen.first_offered_at.is_none_or(|earliest| first.is_earlier(earliest)) {
            seen.first_offered_at = Some(first);
        }

        for moment in &moments {
            if let Some(level) = states.timeline.level_at(moment.t) {
                seen.min_level = Some(seen.min_level.map_or(level, |min| min.min(level)));
            }
            for other in states.log(moment.t).into_iter().filter(|&other| other != quest_id) {
                if let Err(place) = seen.in_log.binary_search(&other) {
                    seen.in_log.insert(place, other);
                }
            }
        }

        // Completions only grow within a session (daily resets aside), so the first and last
        // offer bound the union and the intersection over every offer in between.
        if episode.completed.is_none() {
            return;
        }
        let completed = &mut self.completed;
        let at_first = completed.index_of(episode_sets, states.completed(moments[0].t));
        let at_last = completed.index_of(episode_sets, states.completed(moments[moments.len() - 1].t));
        let of = completed.of_quest.entry(quest_id).or_default();
        push_new(&mut of.at_offers, [at_first, at_last]);

        // Prerequisites only from giver offers: shared and pushed quests skip the giver's checks.
        let mut from_giver = moments.iter().filter(|moment| moment.from_giver).map(|moment| moment.t);
        let Some(first_from_giver) = from_giver.next() else { return };
        let last_from_giver = from_giver.next_back().unwrap_or(first_from_giver);
        let at_first_giver = completed.index_of(episode_sets, states.completed(first_from_giver));
        let at_last_giver = completed.index_of(episode_sets, states.completed(last_from_giver));
        let of = completed.of_quest.entry(quest_id).or_default();
        push_new(of.at_giver_offers.get_or_insert_with(Vec::new), [at_first_giver, at_last_giver]);
    }
}

/// Adds the items `list` lacks, in order.
fn push_new<T: PartialEq>(list: &mut Vec<T>, items: impl IntoIterator<Item = T>) {
    for item in items {
        if !list.contains(&item) {
            list.push(item);
        }
    }
}

impl CharacterOffers {
    /// "Turned in x at giver g, and g offered y within the window": y is handed off by x's turn-in.
    fn record_handoffs(&mut self, episode: &QuestEpisode, window: SessionTime) {
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
                self.handoffs.entry((turn_in.quest_id, next)).or_default().insert(giver);
            }
        }
    }
}

struct OfferIndexBuilder {
    params: AbsenceParams,
    /// Groups same-name copies for `required_any`.
    shared_names: FxHashMap<QuestId, SharedName>,
    index: OfferIndex,
    /// Quest -> quests in every completed-set batch it arrived in, and how many batches that was.
    batch_mates: FxHashMap<QuestId, FxHashSet<QuestId>>,
    batch_counts: FxHashMap<QuestId, usize>,
}

impl OfferIndexBuilder {
    /// Adds the next characters, in the order the buckets list them: the index's insertion orders
    /// follow it, and so does each quest's prerequisite tally. Quests are independent of each
    /// other, so they take their characters side by side, each quest its characters in order.
    fn add(&mut self, characters: impl IntoIterator<Item = (CharacterId, CharacterOffers)>) {
        let OfferIndexBuilder {
            shared_names,
            index,
            batch_mates,
            batch_counts,
            ..
        } = self;
        let mut by_quest: FxHashMap<QuestId, Vec<(CharacterId, OfferedToCharacter)>> = FxHashMap::default();
        for (character, offers) in characters {
            index.episodes += offers.episodes;
            for (quest_id, seen) in offers.quests {
                index.quests.entry(quest_id).or_default();
                by_quest.entry(quest_id).or_default().push((character, seen));
            }
            for (giver, quests) in offers.quests_by_giver {
                index.quests_by_giver.entry(giver).or_default().extend(quests);
            }
            index.co_offered.extend(offers.co_offered);
            for (quests, givers) in offers.handoffs {
                index.handoffs.entry(quests).or_default().extend(givers);
            }
            for (quest_id, givers) in offers.finishers_of {
                index.finishers_of.entry(quest_id).or_default().extend(givers);
            }
            for batch in &offers.completed_batches {
                for &quest_id in batch {
                    *batch_counts.entry(quest_id).or_default() += 1;
                    batch_mates
                        .entry(quest_id)
                        .and_modify(|mates| mates.retain(|mate| batch.contains(mate)))
                        .or_insert_with(|| batch.iter().copied().collect());
                }
            }
        }

        let quests: Vec<_> = index
            .quests
            .iter_mut()
            .filter_map(|(quest_id, offers)| Some((quest_id, offers, by_quest.remove(quest_id)?)))
            .collect();
        quests.into_par_iter().for_each(|(&quest_id, offers, characters)| {
            for (character, seen) in characters {
                offers.add(quest_id, character, seen, shared_names);
            }
        });
    }

    fn build(self) -> OfferIndex {
        let OfferIndexBuilder {
            params,
            mut index,
            batch_mates,
            batch_counts,
            ..
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
        for offers in index.quests.values_mut() {
            (offers.required, offers.required_any) = std::mem::take(&mut offers.prerequisites).prerequisites(params.required_tolerance);
        }
        index
    }
}

/// For one quest: per other quest, how many characters had it completed at every giver offer of
/// this quest, and the same for same-name groups of other quests.
///
/// The TypeScript pipeline lists `required` and `required_any` in the order it meets them when
/// it walks the characters in the order they were first offered the quest by a giver, each
/// character's quests ascending. A character's episodes are together and characters are added in
/// the order the buckets list them, which is that order: each character's rank is the number
/// added before it, and every count keeps the rank of the first character it was met at.
#[derive(Default)]
struct PrerequisiteTally {
    /// Characters added so far: offered the quest by a giver, with a completed history.
    characters: u32,
    completed: FxHashMap<QuestId, Count>,
    groups: FxHashMap<SharedName, GroupCount>,
}

/// Characters counted, and the rank of the first.
#[derive(Clone, Copy, Default)]
struct Count {
    characters: u32,
    first_rank: u32,
}

impl Count {
    /// Counts a character; true when it is the first.
    fn add(&mut self, rank: u32) -> bool {
        let first = self.characters == 0;
        if first {
            self.first_rank = rank;
        }
        self.characters += 1;
        first
    }
}

/// Characters that had any member of a same-name group completed, and the members seen.
#[derive(Default)]
struct GroupCount {
    count: Count,
    /// The smallest member the first character had: the group's place in the list.
    first_member: QuestId,
    members: Vec<QuestId>,
}

impl PrerequisiteTally {
    /// The next character's quests completed at each giver offer (ascending). `shared_names`
    /// holds the quests whose name another catalog quest has: only those can form a group.
    fn add(&mut self, completed: impl IntoIterator<Item = QuestId>, own_name: Option<SharedName>, shared_names: &FxHashMap<QuestId, SharedName>) {
        let rank = self.characters;
        self.characters += 1;
        let mut groups_here: FxHashSet<SharedName> = FxHashSet::default();
        for other in completed {
            self.completed.entry(other).or_default().add(rank);
            let Some(&name) = shared_names.get(&other).filter(|&&name| Some(name) != own_name) else {
                continue;
            };
            let group = self.groups.entry(name).or_default();
            if !group.members.contains(&other) {
                group.members.push(other);
            }
            // Ascending: the first member met is the group's smallest for this character.
            if groups_here.insert(name) && group.count.add(rank) {
                group.first_member = other;
            }
        }
    }

    fn prerequisites(self, tolerance: f64) -> (QuestSet, IndexMap<QuestId, Vec<QuestId>, FxBuildHasher>) {
        let characters = self.characters;
        let enough = characters - (f64::from(characters) * tolerance).floor() as u32;
        let mut required: Vec<(u32, QuestId)> = self
            .completed
            .into_iter()
            .filter(|(_, count)| count.characters >= enough)
            .map(|(quest_id, count)| (count.first_rank, quest_id))
            .collect();
        required.sort_unstable();
        let required: QuestSet = required.into_iter().map(|(_, quest_id)| quest_id).collect();

        let mut groups: Vec<GroupCount> = self
            .groups
            .into_values()
            .filter(|group| group.count.characters >= enough && group.members.len() >= 2)
            .collect();
        groups.sort_unstable_by_key(|group| (group.count.first_rank, group.first_member));
        let required_any = groups
            .into_iter()
            .map(|mut group| {
                group.members.sort_unstable();
                group.members
            })
            .filter(|members| !members.iter().any(|member| required.contains(member)))
            .map(|members| (members[0], members))
            .collect();
        (required, required_any)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode_store::InMemory;
    use crate::model::{CompletedChange, CompletedSource, CompletedTimeline};
    use crate::test_support::{catalog, catalog_quest, gossip, make_episode, npc};

    const X: QuestId = 100;

    fn completed(initial: &[QuestId], changes: &[(f64, &[QuestId])]) -> Option<CompletedTimeline> {
        Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: initial.to_vec(),
            changes: changes
                .iter()
                .map(|&(t, add)| CompletedChange {
                    t,
                    add: Some(add.to_vec()),
                    remove: None,
                })
                .collect(),
        })
    }

    fn index_of(episodes: &[QuestEpisode]) -> OfferIndex {
        let catalog = catalog([catalog_quest(X)]);
        index_offers(&InMemory(episodes), &catalog, AbsenceParams::default(), &mut CharacterIds::default()).unwrap()
    }

    #[test]
    fn lists_required_quests_as_the_first_offered_characters_show_them_when_reading_them_side_by_side() {
        // Twenty characters in one bucket, read side by side, are offered X by its giver. With 10%
        // tolerance a quest two of them lack is still required. The TypeScript lists required quests
        // as it meets them walking the characters in offer order, each one's quests ascending: 30
        // (the first character's only one), 20 (the second's new one), then 10. Characters added in
        // any other order would list them otherwise. The first character comes back in a later
        // session, and is still one character.
        let offered = |character: &str, done: &[QuestId], session_order: u32| QuestEpisode {
            character_key: character.into(),
            session_order,
            completed: completed(done, &[]),
            offers: vec![gossip(5.0, npc(1), &[X], &[])],
            ..make_episode()
        };
        let mut episodes = vec![offered("first", &[30], 0), offered("first", &[30], 1), offered("second", &[20, 30], 0)];
        episodes.extend((0..18).map(|i| offered(&format!("other-{i:02}"), &[10, 20, 30], 0)));

        let index = index_of(&episodes);

        assert_eq!(index.quests[&X].required.iter().copied().collect::<Vec<_>>(), [30, 20, 10]);
        assert_eq!(index.quests[&X].characters(), 20);
    }

    #[test]
    fn keeps_the_order_quests_were_first_offered_in_when_reading_characters_side_by_side() {
        // Each character in one bucket, more than a chunk of them, is offered X and a quest of their
        // own at one giver. Blocked moments are looked for in the order a giver's quests were first
        // offered, so the index must list them in the order the characters come, however they
        // were read.
        let giver = npc(1);
        let characters = super::super::CHARACTERS_PER_CHUNK + 44;
        let own_quests = 1000..1000 + characters as QuestId;
        let episodes: Vec<QuestEpisode> = own_quests
            .clone()
            .map(|own| QuestEpisode {
                offers: vec![gossip(5.0, giver, &[X, own], &[])],
                ..make_episode()
            })
            .collect();

        let index = index_of(&episodes);

        let expected: Vec<QuestId> = [X].into_iter().chain(own_quests).collect();
        assert_eq!(index.quests_by_giver[&giver].iter().copied().collect::<Vec<_>>(), expected);
        assert_eq!(index.quests.keys().copied().collect::<Vec<_>>(), expected);
        assert_eq!(index.episodes, characters);
    }

    #[test]
    fn counts_what_was_completed_at_any_offer_but_requires_what_was_completed_at_every_one() {
        // X is offered before and after P is completed, Z only before. X itself shows as completed
        // (a stale read), which says nothing about what X needs.
        const P: QuestId = 200;
        const Q: QuestId = 300;
        const Z: QuestId = 400;
        let episode = QuestEpisode {
            completed: completed(&[Q, X], &[(20.0, &[P])]),
            offers: vec![gossip(5.0, npc(1), &[X, Z], &[]), gossip(50.0, npc(1), &[X], &[])],
            ..make_episode()
        };

        let index = index_of(&[episode]);

        let completed_when_offered = |quest_id: QuestId| {
            let mut counts: Vec<(QuestId, u32)> = index.quests[&quest_id]
                .completed_when_offered
                .iter()
                .map(|(&other, &count)| (other, count))
                .collect();
            counts.sort_unstable();
            counts
        };
        // Ascending by quest: X (100) < P (200) < Q (300).
        assert_eq!(completed_when_offered(X), [(P, 1), (Q, 1)]);
        assert_eq!(completed_when_offered(Z), [(X, 1), (Q, 1)]);
        assert_eq!(index.quests[&X].required.iter().copied().collect::<Vec<_>>(), [Q]);
        assert_eq!(index.quests[&Z].required.iter().copied().collect::<Vec<_>>(), [X, Q]);
    }
}
