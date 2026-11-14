//! Sightings: moments a character demonstrably could take quest X. The server only offers a
//! quest (gossip/greeting list, accept dialog) or lets it be accepted when every prerequisite is
//! completed, so the completed set at each sighting must contain all of X's prerequisites.
//!
//! One [`CharacterView`] per (quest, character) condenses that character's sightings of X: the
//! quests completed at every sighting (prerequisite candidates), and the timing facts the
//! inference step uses to tell a real prerequisite from a quest everyone happened to finish first.

use crate::eligibility::is_forever_interface;
use crate::model::{GiverKind, GiverRef, PlayerContext, QuestCatalog, QuestEpisode, QuestEventKind, QuestId, SessionTime};
use crate::timeline::{Completion, EpisodeTimeline};
use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SightingKind {
    /// Listed as available by a giver (any offer snapshot, including standalone detail dialogs).
    Offer,
    /// The accept dialog was shown.
    Detail,
    Accepted,
    /// First seen in the quest log without an observed accept (accepted before the capture).
    Log,
}

impl SightingKind {
    pub fn name(self) -> &'static str {
        match self {
            SightingKind::Offer => "offer",
            SightingKind::Detail => "detail",
            SightingKind::Accepted => "accepted",
            SightingKind::Log => "log",
        }
    }
}

/// A moment in one character's sessions: the index into its episodes (session order) and the
/// session time.
#[derive(Clone, Copy, Debug)]
struct Moment {
    episode: usize,
    t: SessionTime,
}

impl Moment {
    fn is_before(self, other: Moment) -> bool {
        self.episode < other.episode || (self.episode == other.episode && self.t < other.t)
    }
}

struct Sighting {
    at: Moment,
    kind: SightingKind,
    giver: Option<GiverRef>,
}

/// A visit to the quest's giver before the character first saw the quest there.
#[derive(Clone, Debug, PartialEq)]
pub struct EarlierVisit {
    pub level: Option<i64>,
    /// The survivors not completed yet at that visit, ascending.
    pub not_completed: Vec<QuestId>,
}

/// One character's sightings of one quest.
#[derive(Debug)]
pub struct CharacterView<'e> {
    pub quest_id: QuestId,
    /// Race and class, to tell "lacked P" from "could never take P" (P is another faction's copy).
    pub player: &'e PlayerContext,
    /// Episode and time of the character's first sighting of the quest.
    pub episode_key: &'e str,
    pub t: SessionTime,
    pub kind: SightingKind,
    pub giver: Option<GiverRef>,
    pub level: Option<i64>,
    /// Seconds since the character last levelled up in the session of the first sighting, if it did.
    pub level_up_gap: Option<f64>,
    /// Catalog quests completed at every sighting, ascending: the prerequisite candidates.
    pub survivors: Vec<QuestId>,
    /// Survivor -> seconds from its completion to the first sighting, for survivors completed
    /// during that session (not already at capture start).
    pub completion_gaps: BTreeMap<QuestId, f64>,
    /// Set when the character interacted with the quest's giver (any list, dialog or turn-in)
    /// before first seeing the quest there. Its `not_completed` survivors became completed while
    /// the character was already at the giver.
    pub arrival: Option<EarlierVisit>,
    /// The last complete offer list from the quest's giver before the first sighting. It did not
    /// offer the quest (that would have been the first sighting), so whatever survivors were
    /// already completed then were not enough on their own; `not_completed` are the ones still
    /// to come.
    pub last_list: Option<EarlierVisit>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveStats {
    pub episodes: usize,
    /// Forever episodes with a completed-quest history, grouped into characters.
    pub usable_episodes: usize,
    pub characters: usize,
    pub skipped_not_forever: usize,
    pub skipped_no_completed: usize,
    /// Episodes whose completed history is empty although the character is past level 2 (an
    /// unloaded history).
    pub skipped_empty_history: usize,
    pub views: usize,
}

/// Quests that can be prerequisite candidates: known to the catalog and not repeatable
/// (completion of a repeatable quest is reset).
pub fn prerequisite_candidates(catalog: &QuestCatalog) -> FxHashSet<QuestId> {
    catalog.quests.values().filter(|quest| !quest.is_repeatable()).map(|quest| quest.id).collect()
}

enum Usability {
    Usable,
    NotForever,
    NoCompleted,
    EmptyHistory,
}

/// The same Forever filter as [`crate::signals::forever_episodes`], as the first of the reasons
/// [`ObserveStats`] counts episodes out for.
fn usability(episode: &QuestEpisode) -> Usability {
    if !is_forever_interface(episode.interface_version) {
        return Usability::NotForever;
    }
    let Some(completed) = &episode.completed else {
        return Usability::NoCompleted;
    };
    let start_level = episode.levels.first().map_or(0, |level| level.v);
    if completed.initial.is_empty() && start_level > 2 {
        return Usability::EmptyHistory;
    }
    Usability::Usable
}

/// Usable episodes grouped by character (in the order characters first appear), each
/// character's sessions oldest first (`session_order`).
pub fn group_by_character<'e>(episodes: &'e [QuestEpisode], stats: &mut ObserveStats) -> IndexMap<&'e str, Vec<&'e QuestEpisode>, FxBuildHasher> {
    let mut characters: IndexMap<&str, Vec<&QuestEpisode>, FxBuildHasher> = IndexMap::default();
    for episode in episodes {
        stats.episodes += 1;
        match usability(episode) {
            Usability::Usable => {}
            Usability::NotForever => {
                stats.skipped_not_forever += 1;
                continue;
            }
            Usability::NoCompleted => {
                stats.skipped_no_completed += 1;
                continue;
            }
            Usability::EmptyHistory => {
                stats.skipped_empty_history += 1;
                continue;
            }
        }
        stats.usable_episodes += 1;
        characters.entry(episode.character_key.as_str()).or_default().push(episode);
    }
    for sessions in characters.values_mut() {
        sessions.sort_by_key(|episode| episode.session_order);
    }
    stats.characters = characters.len();
    characters
}

fn is_place(giver: &GiverRef) -> bool {
    giver.kind != GiverKind::Item
}

/// Every sighting of a catalog quest, per quest, in the order they were recorded: per episode the
/// offers, then the accept dialogs and accepts, then first appearances in the quest log.
fn sightings(episodes: &[&QuestEpisode], catalog: &QuestCatalog) -> BTreeMap<QuestId, Vec<Sighting>> {
    let mut sightings: BTreeMap<QuestId, Vec<Sighting>> = BTreeMap::new();
    let mut add = |quest_id: QuestId, sighting: Sighting| {
        if catalog.quest(quest_id).is_some() {
            sightings.entry(quest_id).or_default().push(sighting);
        }
    };
    for (index, episode) in episodes.iter().enumerate() {
        for offer in &episode.offers {
            for quest in &offer.available {
                let at = Moment { episode: index, t: offer.t };
                add(
                    quest.id,
                    Sighting {
                        at,
                        kind: SightingKind::Offer,
                        giver: offer.giver,
                    },
                );
            }
        }
        for event in &episode.quest_events {
            let kind = match event.kind {
                QuestEventKind::Detail => SightingKind::Detail,
                QuestEventKind::Accepted => SightingKind::Accepted,
                _ => continue,
            };
            add(
                event.quest_id,
                Sighting {
                    at: Moment { episode: index, t: event.t },
                    kind,
                    giver: event.giver,
                },
            );
        }
        let mut seen_in_log = FxHashSet::default();
        for entry in &episode.quest_log {
            for &quest_id in &entry.v {
                if seen_in_log.insert(quest_id) {
                    add(
                        quest_id,
                        Sighting {
                            at: Moment { episode: index, t: entry.t },
                            kind: SightingKind::Log,
                            giver: None,
                        },
                    );
                }
            }
        }
    }
    sightings
}

/// Per giver: the earliest interaction of any kind across the character's sessions, and every
/// complete offer list in session order. Items are skipped: carrying an item says nothing about
/// arriving somewhere.
struct GiverVisits {
    arrivals: FxHashMap<GiverRef, Moment>,
    complete_lists: FxHashMap<GiverRef, Vec<Moment>>,
}

impl GiverVisits {
    fn new(episodes: &[&QuestEpisode]) -> Self {
        let mut visits = GiverVisits {
            arrivals: FxHashMap::default(),
            complete_lists: FxHashMap::default(),
        };
        for (index, episode) in episodes.iter().enumerate() {
            for offer in &episode.offers {
                let at = Moment { episode: index, t: offer.t };
                if let Some(giver) = offer.giver.filter(is_place) {
                    visits.visit(giver, at);
                    if offer.list_complete {
                        visits.complete_lists.entry(giver).or_default().push(at);
                    }
                }
            }
            for event in &episode.quest_events {
                if let Some(giver) = event.giver.filter(is_place) {
                    visits.visit(giver, Moment { episode: index, t: event.t });
                }
            }
        }
        visits
    }

    fn visit(&mut self, giver: GiverRef, at: Moment) {
        let arrival = self.arrivals.entry(giver).or_insert(at);
        if at.is_before(*arrival) {
            *arrival = at;
        }
    }
}

/// The givers whose first visit opens X's "window": the giver seen at the first sighting, else
/// the catalog's starters.
fn window_givers(first: &Sighting, catalog: &QuestCatalog, quest_id: QuestId) -> Vec<GiverRef> {
    if let Some(giver) = first.giver {
        return if is_place(&giver) { vec![giver] } else { Vec::new() };
    }
    let Some(quest) = catalog.quest(quest_id) else {
        return Vec::new();
    };
    let npcs = quest.starters.npcs.iter().map(|&id| GiverRef { kind: GiverKind::Npc, id });
    let objects = quest.starters.objects.iter().map(|&id| GiverRef { kind: GiverKind::Object, id });
    npcs.chain(objects).collect()
}

/// Views for every quest one character was seen able to take. `episodes` are the character's
/// sessions in session order; a completion up to `tolerance` seconds after a sighting still
/// counts, to absorb event ordering jitter.
pub fn observe_character<'e>(episodes: &[&'e QuestEpisode], catalog: &QuestCatalog, candidates: &FxHashSet<QuestId>, tolerance: f64) -> Vec<CharacterView<'e>> {
    let timelines: Vec<EpisodeTimeline> = episodes.iter().map(|episode| EpisodeTimeline::new(episode, true)).collect();
    // Per episode, the candidates completed at some point of it (at capture start or later),
    // ascending: every completed set of the episode is within them.
    let ever_completed: Vec<Vec<QuestId>> = episodes
        .iter()
        .zip(&timelines)
        .map(|(episode, timeline)| {
            let initial = episode.completed.iter().flat_map(|completed| &completed.initial).copied();
            let later = timeline.completions_during_session().into_iter().map(|completion| completion.v);
            let mut ids: Vec<QuestId> = initial.chain(later).filter(|id| candidates.contains(id)).collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        })
        .collect();
    let visits = GiverVisits::new(episodes);

    let mut views = Vec::new();
    for (quest_id, mut list) in sightings(episodes, catalog) {
        // Stable: same-time sightings keep their recorded order.
        list.sort_by(|a, b| a.at.episode.cmp(&b.at.episode).then(a.at.t.total_cmp(&b.at.t)));
        let first = &list[0];
        let first_episode = episodes[first.at.episode];
        let first_timeline = &timelines[first.at.episode];

        // Completion only grows (repeatables are not candidates), so the first sighting's set is
        // the smallest; later sightings can still remove a candidate when the data is inconsistent.
        let survivors: Vec<QuestId> = ever_completed[first.at.episode]
            .iter()
            .copied()
            .filter(|&candidate| candidate != quest_id)
            .filter(|&candidate| list.iter().all(|s| timelines[s.at.episode].is_completed_at(candidate, s.at.t + tolerance)))
            .collect();

        let completion_gaps = survivors
            .iter()
            .filter_map(|&candidate| match first_timeline.completion_time(candidate) {
                Some(Completion::At(completed_at)) => Some((candidate, (first.at.t - completed_at).max(0.0))),
                _ => None,
            })
            .collect();

        let earlier_visit = |at: Moment| {
            let timeline = &timelines[at.episode];
            EarlierVisit {
                level: timeline.level_at(at.t),
                not_completed: survivors
                    .iter()
                    .copied()
                    .filter(|&candidate| !timeline.is_completed_at(candidate, at.t))
                    .collect(),
            }
        };
        let givers = window_givers(first, catalog, quest_id);
        let mut earliest: Option<Moment> = None;
        for arrival in givers.iter().filter_map(|giver| visits.arrivals.get(giver)) {
            if earliest.is_none_or(|known| arrival.is_before(known)) {
                earliest = Some(*arrival);
            }
        }
        let arrival = earliest.filter(|earliest| earliest.is_before(first.at)).map(earlier_visit);

        let mut latest: Option<Moment> = None;
        for &shown in givers.iter().filter_map(|giver| visits.complete_lists.get(giver)).flatten() {
            if shown.is_before(first.at) && latest.is_none_or(|known| known.is_before(shown)) {
                latest = Some(shown);
            }
        }
        let last_list = latest.map(earlier_visit);

        let level_up = first_episode.levels.iter().skip(1).rfind(|level| level.t <= first.at.t);

        views.push(CharacterView {
            quest_id,
            player: &first_episode.player,
            episode_key: &first_episode.key,
            t: first.at.t,
            kind: first.kind,
            giver: first.giver,
            level: first_timeline.level_at(first.at.t),
            level_up_gap: level_up.map(|level| first.at.t - level.t),
            survivors,
            completion_gaps,
            arrival,
            last_list,
        });
    }
    views
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CatalogQuest, CompletedChange, CompletedSource, CompletedTimeline, Finishers, Starters, TimedValue};
    use crate::test_support::{catalog, catalog_quest, event, gossip, make_episode, npc};

    const UNIVERSAL: QuestId = 1;
    const PREREQ: QuestId = 10;
    const QUEST: QuestId = 20;
    const DAILY: QuestId = 30;
    const GIVER: GiverRef = GiverRef { kind: GiverKind::Npc, id: 100 };

    fn test_catalog() -> QuestCatalog {
        catalog([
            catalog_quest(UNIVERSAL),
            CatalogQuest {
                finishers: Finishers {
                    npcs: vec![100],
                    objects: vec![],
                },
                ..catalog_quest(PREREQ)
            },
            CatalogQuest {
                starters: Starters {
                    npcs: vec![100],
                    ..Starters::default()
                },
                ..catalog_quest(QUEST)
            },
            CatalogQuest {
                special_flags: Some(1),
                ..catalog_quest(DAILY)
            },
        ])
    }

    fn history(initial: &[QuestId], changes: &[(f64, &[QuestId])]) -> Option<CompletedTimeline> {
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

    fn view_of<'e>(episodes: &[&'e QuestEpisode], catalog: &QuestCatalog, quest_id: QuestId) -> CharacterView<'e> {
        let candidates = prerequisite_candidates(catalog);
        observe_character(episodes, catalog, &candidates, 0.0)
            .into_iter()
            .find(|view| view.quest_id == quest_id)
            .expect("a view of the quest")
    }

    #[test]
    fn keeps_catalog_non_repeatable_quests_completed_at_the_first_sighting_with_freshness_and_the_giver_window() {
        let catalog = test_catalog();
        let mut episode = make_episode();
        episode.completed = history(&[UNIVERSAL, DAILY, 99999], &[(101.0, &[PREREQ])]);
        episode.offers = vec![gossip(10.0, GIVER, &[PREREQ], &[]), gossip(100.5, GIVER, &[QUEST], &[])];
        // The turn-in event lands before the delayed completed-quest delta and before the next list.
        episode.quest_events = vec![event(100.0, QuestEventKind::TurnedIn, PREREQ, Some(npc(100)))];
        let view = view_of(&[&episode], &catalog, QUEST);
        assert_eq!(view.survivors, [UNIVERSAL, PREREQ]);
        assert_eq!(view.completion_gaps, BTreeMap::from([(PREREQ, 0.5)]));
        assert_eq!(
            view.arrival,
            Some(EarlierVisit {
                level: Some(10),
                not_completed: vec![PREREQ]
            })
        );
    }

    #[test]
    fn drops_a_candidate_the_character_lacked_at_any_sighting_across_sessions_in_session_order() {
        // In-progress exports have no session name; sessionOrder is the only ordering.
        let catalog = test_catalog();
        let mut early = make_episode();
        early.character_key = "c".into();
        early.session_order = 0;
        early.completed = history(&[UNIVERSAL], &[]);
        early.offers = vec![gossip(5.0, GIVER, &[QUEST], &[])];
        let mut late = make_episode();
        late.character_key = "c".into();
        late.session_order = 1;
        late.completed = history(&[UNIVERSAL, PREREQ], &[]);
        late.offers = vec![gossip(5.0, GIVER, &[QUEST], &[])];

        let episodes = [late, early];
        let characters = group_by_character(&episodes, &mut ObserveStats::default());
        let view = view_of(&characters["c"], &catalog, QUEST);
        assert_eq!(view.survivors, [UNIVERSAL]);
        assert_eq!(view.episode_key, episodes[1].key);
    }

    #[test]
    fn treats_a_quest_already_in_the_log_at_capture_start_as_taken_with_the_initial_completed_set() {
        let catalog = test_catalog();
        let mut episode = make_episode();
        episode.completed = history(&[UNIVERSAL], &[(50.0, &[PREREQ])]);
        episode.quest_log = vec![TimedValue { t: 0.0, v: vec![QUEST] }];
        let view = view_of(&[&episode], &catalog, QUEST);
        assert_eq!(view.kind, SightingKind::Log);
        assert_eq!(view.survivors, [UNIVERSAL]);
    }

    #[test]
    fn grouping_skips_non_forever_sessions_sessions_without_history_and_empty_histories_past_level_2() {
        let mut ok = make_episode();
        ok.character_key = "ok".into();
        ok.completed = history(&[UNIVERSAL], &[]);
        let mut fresh_character = make_episode();
        fresh_character.character_key = "fresh-character".into();
        fresh_character.levels = vec![TimedValue { t: 0.0, v: 1 }];
        let mut classic = make_episode();
        classic.interface_version = Some(11507);
        let mut no_history = make_episode();
        no_history.completed = None;
        let mut unloaded = make_episode();
        unloaded.levels = vec![TimedValue { t: 0.0, v: 30 }];

        let mut stats = ObserveStats::default();
        let episodes = [ok, fresh_character, classic, no_history, unloaded];
        let characters = group_by_character(&episodes, &mut stats);
        assert_eq!(characters.keys().copied().collect::<Vec<_>>(), ["ok", "fresh-character"]);
        assert_eq!((stats.skipped_not_forever, stats.skipped_no_completed, stats.skipped_empty_history), (1, 1, 1));
    }
}
