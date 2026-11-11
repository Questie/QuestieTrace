//! Counter-evidence for "X needs P": the character was offered X while P was not completed.
//!
//! Questie's preQuestSingle is an OR list, so an offer without P is only a contradiction when no
//! alternative explains it. Alternatives are structural only: a completed same-name variant of P
//! (faction/race/profession copies), or P being closed to the character by race or class. X's
//! other claimed prerequisites are deliberately not alternatives: a wrong P (say, a quest that
//! happened to be turned in at X's giver just before X auto-opened) would then be excused by the
//! real one. What is left splits by where P was: not started (P is no prerequisite at all; X may
//! be a breadcrumb target or have another way in) or in the log (X needs P taken, not finished).

use super::tally::Pair;
use super::variants::SameNameVariants;
use crate::eligibility::{class_allowed, race_allowed};
use crate::model::{GiverRef, QuestCatalog, QuestEventKind, QuestId, SessionTime};
use crate::timeline::EpisodeTimeline;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CounterKind {
    /// P was neither completed nor taken: P is no prerequisite of X.
    NotStarted,
    /// P was taken but not finished: X may need P taken rather than completed.
    InLog,
    /// A completed same-name copy of P, or P being closed to the character, explains the offer.
    Alternative,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CounterObservation {
    pub kind: CounterKind,
    pub pair: Pair,
    /// When X was offered.
    pub t: SessionTime,
    pub giver: Option<GiverRef>,
}

/// For each quest X, the prerequisites P this signal claims for it.
pub type ClaimedPrerequisites = FxHashMap<QuestId, BTreeSet<QuestId>>;

/// Completed-quest deltas are sampled shortly after the change, so a quest completed without a
/// turn-in event can show up to this late (the same tolerance offer-set uses).
pub const COMPLETION_TOLERANCE_SECONDS: SessionTime = 1.0;

struct OfferMoment {
    quest_id: QuestId,
    t: SessionTime,
    giver: Option<GiverRef>,
}

/// One observation per (pair, kind) per episode: characters are counted later.
pub fn find_counter_evidence(
    timeline: &EpisodeTimeline,
    claimed: &ClaimedPrerequisites,
    variants: &SameNameVariants,
    catalog: &QuestCatalog,
) -> Vec<CounterObservation> {
    let episode = timeline.episode;
    // Without the completed history there is no way to say P was not completed.
    if episode.completed.is_none() {
        return Vec::new();
    }

    let mut turn_in_times: FxHashMap<QuestId, Vec<SessionTime>> = FxHashMap::default();
    for event in episode.quest_events.iter().filter(|event| event.kind == QuestEventKind::TurnedIn) {
        turn_in_times.entry(event.quest_id).or_default().push(event.t);
    }
    let completed_for_offer = |quest_id: QuestId, t: SessionTime| {
        if timeline.is_completed_at(quest_id, t) {
            return true;
        }
        // Turn-in events are exact: an offer just before one (say, the list a player saw on
        // arriving to turn P in) was made while P was still open.
        let end = t + COMPLETION_TOLERANCE_SECONDS;
        if turn_in_times
            .get(&quest_id)
            .is_some_and(|times| times.iter().any(|&turn_in_at| turn_in_at > t && turn_in_at <= end))
        {
            return false;
        }
        timeline.is_completed_at(quest_id, end)
    };

    let mut observations = Vec::new();
    let mut seen: FxHashSet<(Pair, CounterKind)> = FxHashSet::default();
    for offer in offer_moments(timeline) {
        let Some(prerequisites) = claimed.get(&offer.quest_id) else { continue };
        for &prerequisite in prerequisites {
            if completed_for_offer(prerequisite, offer.t) {
                continue;
            }
            let pair = Pair {
                prerequisite,
                quest: offer.quest_id,
            };
            let kind = classify(timeline, catalog, variants, prerequisite, offer.t);
            if seen.insert((pair, kind)) {
                observations.push(CounterObservation {
                    kind,
                    pair,
                    t: offer.t,
                    giver: offer.giver,
                });
            }
        }
    }
    observations
}

/// Every moment the character was offered a quest: offer lists, accept dialogs and accepts, in
/// time order.
fn offer_moments(timeline: &EpisodeTimeline) -> Vec<OfferMoment> {
    let episode = timeline.episode;
    let listed = episode.offers.iter().flat_map(|offer| {
        offer.available.iter().map(|quest| OfferMoment {
            quest_id: quest.id,
            t: offer.t,
            giver: offer.giver,
        })
    });
    let dialogs = episode
        .quest_events
        .iter()
        .filter(|event| matches!(event.kind, QuestEventKind::Detail | QuestEventKind::Accepted))
        .map(|event| OfferMoment {
            quest_id: event.quest_id,
            t: event.t,
            giver: event.giver,
        });
    let mut moments: Vec<OfferMoment> = listed.chain(dialogs).collect();
    moments.sort_by(|a, b| a.t.total_cmp(&b.t));
    moments
}

fn classify(timeline: &EpisodeTimeline, catalog: &QuestCatalog, variants: &SameNameVariants, prerequisite: QuestId, t: SessionTime) -> CounterKind {
    let player = &timeline.episode.player;
    if let Some(quest) = catalog.quest(prerequisite) {
        if !race_allowed(quest.required_races, player.race_id) || !class_allowed(quest.required_classes, player.class_id) {
            return CounterKind::Alternative;
        }
    }
    if variants.of(prerequisite).iter().any(|&copy| timeline.is_completed_at(copy, t)) {
        return CounterKind::Alternative;
    }
    if timeline.is_in_log_at(prerequisite, t) {
        CounterKind::InLog
    } else {
        CounterKind::NotStarted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CatalogQuest, CompletedChange, CompletedSource, CompletedTimeline, PlayerContext, QuestEpisode, TimedValue};
    use crate::test_support::{catalog, catalog_quest, event, gossip, make_episode, npc};

    const P: QuestId = 100;
    const X: QuestId = 200;

    fn counters(episode: &QuestEpisode, quests: Vec<CatalogQuest>) -> Vec<CounterKind> {
        let claimed = ClaimedPrerequisites::from_iter([(X, BTreeSet::from([P]))]);
        let catalog = catalog(quests);
        let variants = SameNameVariants::new(&catalog);
        find_counter_evidence(&EpisodeTimeline::new(episode, true), &claimed, &variants, &catalog)
            .into_iter()
            .map(|o| o.kind)
            .collect()
    }

    fn completed(initial: &[QuestId], changes: Vec<CompletedChange>) -> Option<CompletedTimeline> {
        Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: initial.to_vec(),
            changes,
        })
    }

    fn offered_x_at_10() -> QuestEpisode {
        QuestEpisode {
            offers: vec![gossip(10.0, npc(1), &[X], &[])],
            ..make_episode()
        }
    }

    #[test]
    fn offers_without_p_split_by_whether_p_was_not_started_or_still_in_the_log() {
        let in_log = QuestEpisode {
            quest_log: vec![TimedValue { t: 0.0, v: vec![P] }],
            quest_events: vec![event(10.0, QuestEventKind::Detail, X, Some(npc(1)))],
            ..make_episode()
        };
        assert_eq!(counters(&offered_x_at_10(), vec![]), [CounterKind::NotStarted]);
        assert_eq!(counters(&in_log, vec![]), [CounterKind::InLog]);
    }

    #[test]
    fn silent_once_p_is_completed_counting_a_turn_in_just_before_the_offer() {
        let done = QuestEpisode {
            completed: completed(&[P], vec![]),
            ..offered_x_at_10()
        };
        let just_turned_in = QuestEpisode {
            quest_log: vec![TimedValue { t: 0.0, v: vec![P] }],
            quest_events: vec![
                event(10.0, QuestEventKind::TurnedIn, P, Some(npc(1))),
                event(10.2, QuestEventKind::Detail, X, Some(npc(1))),
            ],
            ..make_episode()
        };
        assert_eq!(counters(&done, vec![]), []);
        assert_eq!(counters(&just_turned_in, vec![]), []);
    }

    #[test]
    fn a_completion_delta_just_after_the_offer_is_tolerated_but_a_turn_in_event_is_not() {
        let late_delta = QuestEpisode {
            completed: completed(
                &[],
                vec![CompletedChange {
                    t: 10.6,
                    add: Some(vec![P]),
                    remove: None,
                }],
            ),
            ..offered_x_at_10()
        };
        let before_turn_in = QuestEpisode {
            quest_log: vec![TimedValue { t: 0.0, v: vec![P] }],
            quest_events: vec![event(10.5, QuestEventKind::TurnedIn, P, Some(npc(1)))],
            ..offered_x_at_10()
        };
        assert_eq!(counters(&late_delta, vec![]), []);
        assert_eq!(counters(&before_turn_in, vec![]), [CounterKind::InLog]);
    }

    #[test]
    fn a_completed_same_name_copy_or_p_being_closed_to_the_race_explains_the_offer() {
        let copies = || {
            vec![
                CatalogQuest {
                    name: Some("Intro".into()),
                    ..catalog_quest(P)
                },
                CatalogQuest {
                    name: Some("Intro".into()),
                    ..catalog_quest(101)
                },
            ]
        };
        let via_copy = QuestEpisode {
            completed: completed(&[101], vec![]),
            ..offered_x_at_10()
        };
        assert_eq!(counters(&via_copy, copies()), [CounterKind::Alternative]);
        assert_eq!(counters(&offered_x_at_10(), copies()), [CounterKind::NotStarted], "the copy must be completed");

        let orc = QuestEpisode {
            player: PlayerContext {
                race_id: Some(2),
                ..PlayerContext::default()
            },
            ..offered_x_at_10()
        };
        let human_only = CatalogQuest {
            required_races: Some(1),
            ..catalog_quest(P)
        };
        assert_eq!(counters(&orc, vec![human_only]), [CounterKind::Alternative]);
    }

    #[test]
    fn says_nothing_without_a_completed_quest_history() {
        let episode = QuestEpisode {
            completed: None,
            ..offered_x_at_10()
        };
        assert_eq!(counters(&episode, vec![]), []);
    }
}
