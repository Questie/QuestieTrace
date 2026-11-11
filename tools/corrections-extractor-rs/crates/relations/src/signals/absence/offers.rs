//! Pass 1 of the absence signal: everything known from moments a quest WAS available.
//!
//! A blocked moment (blocked.rs) only says "something stopped X here". What that something can
//! be is decided by contrast with every moment X was offered to anyone: a quest that sat in some
//! character's log while X was still offered cannot be what hides X while in the log, and so on.
//! This pass also learns which givers really start X (as observed, not as the catalog claims),
//! which quests are listed side by side, and which turn-ins hand off to which offers.
//!
//! Later passes only ask how many characters saw a state while X was offered, so a character's
//! offers are tallied once all of their episodes are read (ingest lists a character's episodes
//! together): only the characters being read keep their states.

use super::params::AbsenceParams;
use super::states::{EpisodeStates, QuestSet};
use crate::model::{CompletedTimeline, Faction, GiverRef, QuestCatalog, QuestEpisode, QuestEventKind, QuestId, SessionTime};
use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::rc::Rc;

/// A character, by its pseudonymous `QuestEpisode::character_key`.
pub type Character<'a> = &'a str;

/// What was observed while one quest was available.
#[derive(Default)]
pub struct QuestOffers<'a> {
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
    pub first_offered_at: FxHashMap<Character<'a>, Moment>,
    /// What `required` and `required_any` are made of once every character is tallied.
    prerequisites: PrerequisiteTally<'a>,
}

impl QuestOffers<'_> {
    /// Characters the quest was offered to.
    pub fn characters(&self) -> usize {
        self.first_offered_at.len()
    }
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

/// Pass 1 over `episodes` (in file order: insertion orders follow it).
pub fn index_offers<'a>(episodes: &[&'a QuestEpisode], catalog: &'a QuestCatalog, params: AbsenceParams) -> OfferIndex<'a> {
    let last_episode_of: FxHashMap<Character, usize> = episodes.iter().enumerate().map(|(i, episode)| (episode.character_key.as_str(), i)).collect();
    let mut builder = OfferIndexBuilder {
        params,
        shared_names: shared_names(catalog),
        index: OfferIndex::default(),
        reading: FxHashMap::default(),
        batch_mates: FxHashMap::default(),
        batch_counts: FxHashMap::default(),
    };
    for (i, &episode) in episodes.iter().enumerate() {
        builder.add(episode);
        // Any episode order gives the same index; with ingest's (a character's episodes together)
        // only one character is being read at a time.
        if last_episode_of[episode.character_key.as_str()] == i {
            builder.tally_character(&episode.character_key);
        }
    }
    builder.build()
}

/// The names of catalog quests that share their name with another one: Forever's zone and race
/// copies. Only these can form a same-name group of two or more.
fn shared_names(catalog: &QuestCatalog) -> FxHashMap<QuestId, &str> {
    let mut quests_named: FxHashMap<&str, usize> = FxHashMap::default();
    for name in catalog.quests.values().filter_map(|quest| quest.name.as_deref()) {
        *quests_named.entry(name).or_default() += 1;
    }
    catalog
        .quests
        .iter()
        .filter_map(|(&id, quest)| quest.name.as_deref().filter(|name| quests_named[name] >= 2).map(|name| (id, name)))
        .collect()
}

struct OfferMoment {
    t: SessionTime,
    /// Reached through a known giver. Giverless details are party shares and pushed quests.
    from_giver: bool,
}

/// What one character's episodes read so far showed about one quest offered to them.
struct CharacterOffers {
    first_offered_at: Moment,
    in_log: FxHashSet<QuestId>,
    completed: FxHashSet<QuestId>,
    /// The character's rank among those offered the quest by a giver, and the quests completed
    /// at each of those offers (ascending).
    completed_at_each_giver_offer: Option<(u32, Vec<QuestId>)>,
}

struct OfferIndexBuilder<'a> {
    params: AbsenceParams,
    /// Groups same-name copies for `required_any`.
    shared_names: FxHashMap<QuestId, &'a str>,
    index: OfferIndex<'a>,
    /// Per character whose episodes are being read, per quest offered to them.
    reading: FxHashMap<Character<'a>, FxHashMap<QuestId, CharacterOffers>>,
    /// Quest -> quests in every completed-set batch it arrived in, and how many batches that was.
    batch_mates: FxHashMap<QuestId, FxHashSet<QuestId>>,
    batch_counts: FxHashMap<QuestId, usize>,
}

impl<'a> OfferIndexBuilder<'a> {
    fn add(&mut self, episode: &'a QuestEpisode) {
        self.index.episodes += 1;
        let mut states = EpisodeStates::new(episode, &self.params);
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
        if let Some(completed) = &states.completed_history {
            self.record_batches(completed);
        }
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

    fn record_offers(&mut self, episode: &'a QuestEpisode, states: &mut EpisodeStates, quest_id: QuestId, mut moments: Vec<OfferMoment>) {
        let offers = self.index.quests.entry(quest_id).or_default();
        let player = &episode.player;
        if let Some(faction) = player.faction.filter(|faction| !offers.factions.contains(faction)) {
            offers.factions.push(faction);
        }
        offers.races.extend(player.race_id);
        offers.classes.extend(player.class_id);

        moments.sort_by(|a, b| a.t.total_cmp(&b.t));
        let first = Moment::of(episode, moments[0].t);
        let seen = self
            .reading
            .entry(episode.character_key.as_str())
            .or_default()
            .entry(quest_id)
            .or_insert_with(|| CharacterOffers {
                first_offered_at: first,
                in_log: FxHashSet::default(),
                completed: FxHashSet::default(),
                completed_at_each_giver_offer: None,
            });
        if first.is_earlier(seen.first_offered_at) {
            seen.first_offered_at = first;
        }

        for moment in &moments {
            if let Some(level) = states.timeline.level_at(moment.t) {
                offers.min_level = Some(offers.min_level.map_or(level, |min| min.min(level)));
            }
            seen.in_log.extend(states.log(moment.t).into_iter().filter(|&other| other != quest_id));
        }

        // Completions only grow within a session (daily resets aside), so the first and last
        // offer bound the union and the intersection over every offer in between.
        if episode.completed.is_none() {
            return;
        }
        let at_first = states.completed(moments[0].t);
        let at_last = states.completed(moments[moments.len() - 1].t);
        seen.completed.extend(at_last.iter().copied().filter(|&other| other != quest_id));
        if !Rc::ptr_eq(&at_first, &at_last) {
            seen.completed.extend(at_first.iter().copied().filter(|&other| other != quest_id));
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
        match &mut seen.completed_at_each_giver_offer {
            Some((_, earlier)) => *earlier = intersect_sorted(earlier, &completed),
            None => seen.completed_at_each_giver_offer = Some((offers.prerequisites.rank_next(), completed)),
        }
    }

    /// Adds a character whose episodes have all been read to the counts of every quest offered to them.
    fn tally_character(&mut self, character: &str) {
        let Some((character, quests)) = self.reading.remove_entry(character) else {
            return;
        };
        for (quest_id, seen) in quests {
            let offers = &mut self.index.quests[&quest_id];
            offers.first_offered_at.insert(character, seen.first_offered_at);
            for other in seen.in_log {
                *offers.in_log_when_offered.entry(other).or_default() += 1;
            }
            for other in seen.completed {
                *offers.completed_when_offered.entry(other).or_default() += 1;
            }
            if let Some((rank, completed)) = seen.completed_at_each_giver_offer {
                offers
                    .prerequisites
                    .add(rank, &completed, self.shared_names.get(&quest_id).copied(), &self.shared_names);
            }
        }
    }

    fn build(self) -> OfferIndex<'a> {
        let OfferIndexBuilder {
            params,
            mut index,
            reading,
            batch_mates,
            batch_counts,
            ..
        } = self;
        debug_assert!(reading.is_empty(), "every character is tallied after their last episode");
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

    fn record_batches(&mut self, completed: &CompletedTimeline) {
        for change in &completed.changes {
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

/// For one quest: per other quest, how many characters had it completed at every giver offer of
/// this quest, and the same for same-name groups of other quests.
///
/// The TypeScript pipeline lists `required` and `required_any` in the order it meets them when
/// it walks the characters in the order they were first offered the quest by a giver, each
/// character's quests ascending. Characters are tallied when their episodes are all read, so
/// each one carries that rank, and every count keeps the smallest (rank, quest) it was met at.
#[derive(Default)]
struct PrerequisiteTally<'a> {
    /// Characters ranked so far: offered the quest by a giver, with a completed history.
    characters: u32,
    completed: FxHashMap<QuestId, Count>,
    groups: FxHashMap<&'a str, GroupCount>,
}

/// Characters counted, and the best (smallest) rank among them.
#[derive(Clone, Copy, Default)]
struct Count {
    characters: u32,
    first_rank: u32,
}

impl Count {
    /// Counts a character; true when its rank is the best so far.
    fn add(&mut self, rank: u32) -> bool {
        let first = self.characters == 0 || rank < self.first_rank;
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
    /// The smallest member the best-ranked character had: the group's place in the list.
    first_member: QuestId,
    members: Vec<QuestId>,
}

impl<'a> PrerequisiteTally<'a> {
    /// The rank of a character first offered the quest by a giver now.
    fn rank_next(&mut self) -> u32 {
        self.characters += 1;
        self.characters - 1
    }

    /// One character's quests completed at each giver offer (ascending). `shared_names` holds
    /// the quests whose name another catalog quest has: only those can form a group.
    fn add(&mut self, rank: u32, completed: &[QuestId], own_name: Option<&str>, shared_names: &FxHashMap<QuestId, &'a str>) {
        let mut groups_here: FxHashSet<&str> = FxHashSet::default();
        for &other in completed {
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
    use crate::model::CompletedSource;
    use crate::test_support::{catalog, catalog_quest, gossip, make_episode, npc};

    #[test]
    fn lists_required_quests_as_the_first_offered_character_shows_them_even_when_tallied_last() {
        // Ten characters are offered X by its giver. The first one offered it had only 30 done, the
        // nine others 20 and 30: with 10% tolerance both are required. The TypeScript lists them as
        // it meets them walking the characters in offer order: 30, then 20. The first character has
        // a later episode too, so they are tallied last.
        const X: QuestId = 100;
        let offered = |character: &str, done: &[QuestId], session_order: u32| QuestEpisode {
            character_key: character.into(),
            session_order,
            completed: Some(CompletedTimeline {
                source: CompletedSource::Modern,
                initial: done.to_vec(),
                changes: vec![],
            }),
            offers: vec![gossip(5.0, npc(1), &[X], &[])],
            ..make_episode()
        };
        let mut episodes = vec![offered("first", &[30], 0)];
        episodes.extend((0..9).map(|i| offered(&format!("other-{i}"), &[20, 30], 0)));
        episodes.push(offered("first", &[30], 1));
        let catalog = catalog([catalog_quest(X)]);

        let index = index_offers(&episodes.iter().collect::<Vec<_>>(), &catalog, AbsenceParams::default());

        assert_eq!(index.quests[&X].required.iter().copied().collect::<Vec<_>>(), [30, 20]);
        assert_eq!(index.quests[&X].characters(), 10);
    }
}
