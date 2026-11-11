//! Finds hand-offs in one episode: the character turns in P at giver G, and G then offers X.
//!
//! Two shapes, from strongest to weakest:
//! - pop: G shows X's accept dialog right after the turn-in, without the player picking X from a
//!   list. Servers do this for the turned-in quest's NextQuestInChain when the same giver starts
//!   it. On Forever a giver whose only available quest is X also opens it directly, so a pop shows
//!   that X was available, and only suggests nextQuestInChain(P) = X (see [`super::candidates`]).
//! - list: G's complete offer list after the turn-in contains X, while G's complete list before it
//!   did not. Servers leave unavailable quests out, so X became available in between. Forever
//!   usually re-sends G's list 0.2-0.5s after a turn-in that pops nothing, so most of these lists
//!   are immediate. P's completion is one explanation; anything else that changed in between (a
//!   level-up, another completion, a quest leaving the log) is recorded as a [`Confounder`].

use crate::model::{CatalogQuest, GiverRef, OfferSnapshot, OfferSource, QuestCatalog, QuestEpisode, QuestEventKind, QuestId, SessionTime};
use crate::timeline::EpisodeTimeline;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffParams {
    /// Latest an auto-offered accept dialog may follow its turn-in, in seconds. Forever pops arrive
    /// 0.1-0.5s after the turn-in; a thin tail of lagged pops reaches about 3s.
    pub pop_window: f64,
    /// Oldest the "before" list may be, in seconds before the turn-in.
    pub list_before: f64,
    /// Latest the "after" list may be, in seconds after the turn-in.
    pub list_after: f64,
}

/// Delays around a turn-in at a known giver, in seconds, for reports/handoff-timing.json.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurnInTiming {
    /// To the next accept dialog at the same giver with no offer list in between.
    pub next_detail: Option<f64>,
    /// Since the giver's last complete offer list.
    pub previous_list: Option<f64>,
    /// To the giver's next complete offer list.
    pub next_list: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TurnIn {
    pub quest_id: QuestId,
    pub t: SessionTime,
    /// From the turn-in event, or else from the quest's completion dialog just before it.
    pub giver: Option<GiverRef>,
    pub timing: TurnInTiming,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HandoffKind {
    /// G opened X's accept dialog right after the turn-in.
    Pop,
    /// G's complete list after the turn-in offered X; its list at `previous_list_at` did not.
    List { previous_list_at: SessionTime },
}

/// Another change around a hand-off that could also explain X being offered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Confounder {
    /// Another quest turned in at the same giver within the pop window.
    AlsoTurnedIn(QuestId),
    AlsoCompleted(QuestId),
    /// A daily/weekly reset un-completed it, which can reopen quests it blocked (exclusiveTo).
    Reset(QuestId),
    /// Abandoned or otherwise gone without being completed.
    LeftLog(QuestId),
    /// Some quests open only while another is taken (parentQuest, availableStartingWith).
    EnteredLog(QuestId),
    /// Only recorded when X's catalog level does not rule the level-up out.
    LevelUp {
        from: i64,
        to: i64,
    },
}

impl fmt::Display for Confounder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Confounder::AlsoTurnedIn(id) => write!(f, "also turned in {id}"),
            Confounder::AlsoCompleted(id) => write!(f, "also completed {id}"),
            Confounder::Reset(id) => write!(f, "{id} reset"),
            Confounder::LeftLog(id) => write!(f, "{id} left the log"),
            Confounder::EnteredLog(id) => write!(f, "{id} entered the log"),
            Confounder::LevelUp { from, to } => write!(f, "level {from} -> {to}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HandoffObservation {
    pub kind: HandoffKind,
    /// P: the quest that was turned in.
    pub turned_in: QuestId,
    /// X: the quest the giver offered afterwards.
    pub offered: QuestId,
    pub giver: GiverRef,
    pub turn_in_at: SessionTime,
    pub offered_at: SessionTime,
    /// Empty means a clean observation.
    pub confounders: Vec<Confounder>,
}

pub struct EpisodeHandoffs {
    /// Every turn-in in time order, with its timing filled in when it had a giver.
    pub turn_ins: Vec<TurnIn>,
    pub observations: Vec<HandoffObservation>,
    /// Quests on compared before/after lists, newly offered or not: the signal judged them.
    pub compared_quests: BTreeSet<QuestId>,
}

/// How old a completion dialog may be for its giver to stand in for a turn-in event's missing one.
const COMPLETION_DIALOG_MAX_AGE: SessionTime = 120.0;


fn is_giver_list(offer: &OfferSnapshot) -> bool {
    matches!(offer.source, OfferSource::Gossip | OfferSource::Greeting) && offer.giver.is_some()
}

pub fn find_handoffs(timeline: &EpisodeTimeline, catalog: &QuestCatalog, params: &HandoffParams) -> EpisodeHandoffs {
    let episode = timeline.episode;
    let mut turn_ins = resolve_turn_ins(episode);
    let any_lists: Vec<&OfferSnapshot> = episode.offers.iter().filter(|offer| is_giver_list(offer)).collect();
    let lists: Vec<&OfferSnapshot> = any_lists.iter().copied().filter(|offer| offer.list_complete).collect();
    let repeatable_in_lists = repeatable_in_episode(episode);
    let is_repeatable_quest = |quest_id: QuestId| repeatable_in_lists.contains(&quest_id) || catalog.quest(quest_id).is_some_and(CatalogQuest::is_repeatable);
    let mut observations = Vec::new();
    let mut compared_quests = BTreeSet::new();

    for index in 0..turn_ins.len() {
        let TurnIn { quest_id, t, giver, .. } = turn_ins[index];
        let Some(giver) = giver else { continue };
        let before = last_list_before(&lists, giver, t);
        let after = first_list_after(&lists, giver, t);
        // Any list from the giver, even one ingest could not read completely, means the player picked.
        let pop = next_detail(episode, quest_id, t, giver, first_list_after(&any_lists, giver, t));
        turn_ins[index].timing = TurnInTiming {
            next_detail: pop.map(|pop| pop.t - t),
            previous_list: before.map(|before| t - before.t),
            next_list: after.map(|after| after.t - t),
        };
        if is_repeatable_quest(quest_id) {
            continue;
        }

        if let Some(pop) = pop.filter(|pop| pop.t - t <= params.pop_window && is_new_offer(timeline, pop.quest_id, t)) {
            let from = pop.t - params.pop_window;
            let confounders = turn_ins
                .iter()
                .enumerate()
                .filter(|&(other, turn_in)| other != index && turn_in.t >= from && turn_in.t <= pop.t && turn_in.giver == Some(giver))
                .map(|(_, other)| Confounder::AlsoTurnedIn(other.quest_id))
                .collect();
            observations.push(HandoffObservation {
                kind: HandoffKind::Pop,
                turned_in: quest_id,
                offered: pop.quest_id,
                giver,
                turn_in_at: t,
                offered_at: pop.t,
                confounders,
            });
        }

        if let (Some(before), Some(after)) = (before, after) {
            if t - before.t <= params.list_before && after.t - t <= params.list_after {
                let turn_in = &turn_ins[index];
                observations.extend(list_handoffs(timeline, catalog, &is_repeatable_quest, turn_in, giver, before, after));
                compared_quests.extend(before.available.iter().chain(&after.available).map(|quest| quest.id));
            }
        }
    }
    EpisodeHandoffs {
        turn_ins,
        observations,
        compared_quests,
    }
}

/// Turn-ins in time order, with the giver filled in from the completion dialog when the event lacks one.
fn resolve_turn_ins(episode: &QuestEpisode) -> Vec<TurnIn> {
    let mut turn_ins = Vec::new();
    let mut last_dialog: FxHashMap<QuestId, (SessionTime, GiverRef)> = FxHashMap::default();
    for event in &episode.quest_events {
        match (event.kind, event.giver) {
            (QuestEventKind::Complete | QuestEventKind::Progress, Some(giver)) => {
                last_dialog.insert(event.quest_id, (event.t, giver));
            }
            (QuestEventKind::TurnedIn, own) => {
                let from_dialog = || {
                    let &(dialog_t, giver) = last_dialog.get(&event.quest_id)?;
                    (event.t - dialog_t <= COMPLETION_DIALOG_MAX_AGE).then_some(giver)
                };
                turn_ins.push(TurnIn {
                    quest_id: event.quest_id,
                    t: event.t,
                    giver: own.or_else(from_dialog),
                    timing: TurnInTiming::default(),
                });
            }
            _ => {}
        }
    }
    turn_ins
}

/// Quests any offer list in this episode flagged repeatable, daily or weekly.
fn repeatable_in_episode(episode: &QuestEpisode) -> FxHashSet<QuestId> {
    episode
        .offers
        .iter()
        .flat_map(|offer| &offer.available)
        .filter(|quest| quest.repeatable == Some(true) || quest.frequency.unwrap_or(1) > 1)
        .map(|quest| quest.id)
        .collect()
}

fn last_list_before<'o>(lists: &[&'o OfferSnapshot], giver: GiverRef, t: SessionTime) -> Option<&'o OfferSnapshot> {
    lists.iter().rev().find(|list| list.t < t && list.giver == Some(giver)).copied()
}

fn first_list_after<'o>(lists: &[&'o OfferSnapshot], giver: GiverRef, t: SessionTime) -> Option<&'o OfferSnapshot> {
    lists.iter().find(|list| list.t > t && list.giver == Some(giver)).copied()
}

#[derive(Clone, Copy)]
struct Pop {
    quest_id: QuestId,
    t: SessionTime,
}

/// The first accept dialog at the turn-in's giver after the turn-in, if nothing else happened at
/// that giver first. A dialog after a fresh list was picked by the player, and one after another
/// turn-in dialog belongs to that turn-in.
fn next_detail(episode: &QuestEpisode, turned_in: QuestId, t: SessionTime, giver: GiverRef, next_list: Option<&OfferSnapshot>) -> Option<Pop> {
    for event in episode.quest_events.iter().filter(|event| event.t > t) {
        if next_list.is_some_and(|list| event.t > list.t) {
            return None;
        }
        if event.quest_id == turned_in || event.giver != Some(giver) {
            continue;
        }
        match event.kind {
            QuestEventKind::Detail => {
                return Some(Pop {
                    quest_id: event.quest_id,
                    t: event.t,
                })
            }
            QuestEventKind::Progress | QuestEventKind::Complete | QuestEventKind::TurnedIn => return None,
            QuestEventKind::Accepted | QuestEventKind::Abandoned | QuestEventKind::Removed => {}
        }
    }
    None
}

/// False when the quest was already completed or in the log, i.e. the dialog is not a fresh offer.
fn is_new_offer(timeline: &EpisodeTimeline, quest_id: QuestId, t: SessionTime) -> bool {
    !timeline.is_completed_at(quest_id, t) && !timeline.is_in_log_at(quest_id, t)
}

fn list_handoffs(
    timeline: &EpisodeTimeline,
    catalog: &QuestCatalog,
    is_repeatable_quest: &impl Fn(QuestId) -> bool,
    turn_in: &TurnIn,
    giver: GiverRef,
    before: &OfferSnapshot,
    after: &OfferSnapshot,
) -> Vec<HandoffObservation> {
    let previously_offered: FxHashSet<QuestId> = before.available.iter().map(|quest| quest.id).collect();
    let shared = shared_confounders(timeline, turn_in, before.t, after.t);
    let level_before = timeline.level_at(before.t);
    let level_after = timeline.level_at(after.t);
    let mut observations = Vec::new();

    for quest_id in after.available.iter().map(|quest| quest.id) {
        if quest_id == turn_in.quest_id || previously_offered.contains(&quest_id) {
            continue;
        }
        // Hidden before because it was already taken or done, not because it was locked.
        if timeline.is_in_log_at(quest_id, before.t) || timeline.is_completed_at(quest_id, before.t) {
            continue;
        }
        // Repeatable quests come back on their own (after a turn-in or a daily/weekly reset).
        if is_repeatable_quest(quest_id) {
            continue;
        }

        let mut confounders = shared.clone();
        if let (Some(from), Some(to)) = (level_before, level_after) {
            let required_level = catalog.quest(quest_id).and_then(|quest| quest.required_level);
            if to > from && required_level.is_none_or(|required| required > from) {
                confounders.push(Confounder::LevelUp { from, to });
            }
        }
        observations.push(HandoffObservation {
            kind: HandoffKind::List { previous_list_at: before.t },
            turned_in: turn_in.quest_id,
            offered: quest_id,
            giver,
            turn_in_at: turn_in.t,
            offered_at: after.t,
            confounders,
        });
    }
    observations
}

/// Changes between two lists, other than this turn-in, that could unlock any quest.
fn shared_confounders(timeline: &EpisodeTimeline, turn_in: &TurnIn, from: SessionTime, to: SessionTime) -> Vec<Confounder> {
    let episode = timeline.episode;
    let between = |t: SessionTime| t > from && t <= to;
    let mut confounders: Vec<Confounder> = timeline
        .completions_during_session()
        .into_iter()
        .filter(|completion| between(completion.t) && completion.v != turn_in.quest_id)
        .map(|completion| Confounder::AlsoCompleted(completion.v))
        .collect();
    for change in episode
        .completed
        .iter()
        .flat_map(|completed| &completed.changes)
        .filter(|change| between(change.t))
    {
        confounders.extend(change.remove.iter().flatten().map(|&quest_id| Confounder::Reset(quest_id)));
    }
    for &quest_id in log_list_at(episode, from) {
        if quest_id == turn_in.quest_id || timeline.is_in_log_at(quest_id, to) || timeline.is_completed_at(quest_id, to) {
            continue;
        }
        confounders.push(Confounder::LeftLog(quest_id));
    }
    for &quest_id in log_list_at(episode, to) {
        if !timeline.is_in_log_at(quest_id, from) {
            confounders.push(Confounder::EnteredLog(quest_id));
        }
    }
    confounders
}

/// The quest log at `t` as recorded (ingest writes each entry deduplicated and ascending), for
/// listing confounders in a stable order; the timeline's sets answer membership.
fn log_list_at(episode: &QuestEpisode, t: SessionTime) -> &[QuestId] {
    match episode.quest_log.partition_point(|entry| entry.t <= t).checked_sub(1) {
        Some(index) => &episode.quest_log[index].v,
        None => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CompletedChange, CompletedSource, CompletedTimeline, GiverKind, OfferedQuest, QuestEvent, TimedValue};
    use crate::test_support::{catalog, catalog_quest, event, gossip, make_episode, npc};

    const PARAMS: HandoffParams = HandoffParams {
        pop_window: 3.0,
        list_before: 300.0,
        list_after: 300.0,
    };
    const P: QuestId = 100;
    const X: QuestId = 200;
    const G: GiverRef = GiverRef { kind: GiverKind::Npc, id: 1 };

    fn detect(episode: &QuestEpisode, quests: Vec<CatalogQuest>) -> EpisodeHandoffs {
        find_handoffs(&EpisodeTimeline::new(episode, true), &catalog(quests), &PARAMS)
    }

    fn handoffs(episode: &QuestEpisode) -> Vec<HandoffObservation> {
        detect(episode, vec![]).observations
    }

    fn log(entries: &[(f64, &[QuestId])]) -> Vec<TimedValue<Vec<QuestId>>> {
        entries.iter().map(|&(t, ids)| TimedValue { t, v: ids.to_vec() }).collect()
    }

    /// P in the log until it is turned in at G at t = 100, followed by `events`.
    fn turn_in_episode(events: Vec<QuestEvent>) -> QuestEpisode {
        let mut episode = make_episode();
        episode.quest_log = log(&[(0.0, &[P]), (100.2, &[])]);
        episode.quest_events = [
            event(99.0, QuestEventKind::Complete, P, Some(G)),
            event(100.0, QuestEventKind::TurnedIn, P, Some(G)),
        ]
        .into_iter()
        .chain(events)
        .collect();
        episode
    }

    fn summary(observations: &[HandoffObservation]) -> Vec<(HandoffKind, QuestId, QuestId)> {
        observations.iter().map(|o| (o.kind, o.turned_in, o.offered)).collect()
    }

    #[test]
    fn an_accept_dialog_at_the_same_giver_right_after_the_turn_in_is_a_clean_pop() {
        let found = detect(&turn_in_episode(vec![event(100.4, QuestEventKind::Detail, X, Some(G))]), vec![]);
        assert_eq!(summary(&found.observations), [(HandoffKind::Pop, P, X)]);
        assert!(found.observations[0].confounders.is_empty());
        assert_eq!(found.turn_ins[0].timing.next_detail.map(|d| (d * 10.0).round()), Some(4.0));
    }

    #[test]
    fn the_completion_dialog_lends_its_giver_to_a_turn_in_event_without_one() {
        let mut episode = make_episode();
        episode.quest_events = vec![
            event(99.0, QuestEventKind::Complete, P, Some(G)),
            event(100.0, QuestEventKind::TurnedIn, P, None),
            event(100.4, QuestEventKind::Detail, X, Some(G)),
        ];
        let observations = handoffs(&episode);
        assert_eq!(summary(&observations), [(HandoffKind::Pop, P, X)]);
        assert_eq!(observations[0].giver, G);
    }

    #[test]
    fn dialogs_picked_from_a_fresh_list_at_another_giver_or_too_late_are_no_pops() {
        let mut from_list = turn_in_episode(vec![event(102.0, QuestEventKind::Detail, X, Some(G))]);
        from_list.offers = vec![gossip(101.0, G, &[X], &[])];
        let other_giver = turn_in_episode(vec![event(100.4, QuestEventKind::Detail, X, Some(npc(2)))]);
        let late = turn_in_episode(vec![event(110.0, QuestEventKind::Detail, X, Some(G))]);
        assert!(handoffs(&from_list).iter().all(|o| o.kind != HandoffKind::Pop));
        assert_eq!(handoffs(&other_giver), []);
        assert_eq!(handoffs(&late), []);
    }

    #[test]
    fn a_dialog_after_an_incomplete_list_from_the_giver_was_picked_by_the_player() {
        let mut episode = turn_in_episode(vec![event(100.6, QuestEventKind::Detail, X, Some(G))]);
        episode.offers = vec![OfferSnapshot {
            list_complete: false,
            ..gossip(100.2, G, &[X], &[])
        }];
        assert_eq!(handoffs(&episode), []);
    }

    #[test]
    fn a_pop_belongs_to_the_latest_turn_in_at_the_giver() {
        let episode = turn_in_episode(vec![
            event(101.0, QuestEventKind::Complete, 101, Some(G)),
            event(101.5, QuestEventKind::TurnedIn, 101, Some(G)),
            event(101.7, QuestEventKind::Detail, X, Some(G)),
        ]);
        let observations = handoffs(&episode);
        assert_eq!(summary(&observations), [(HandoffKind::Pop, 101, X)]);
        assert_eq!(
            observations[0].confounders,
            [Confounder::AlsoTurnedIn(P)],
            "P was turned in at G within the pop window"
        );
    }

    #[test]
    fn repeatable_quests_are_no_prerequisites() {
        let episode = turn_in_episode(vec![event(100.4, QuestEventKind::Detail, X, Some(G))]);
        let repeatable = CatalogQuest {
            special_flags: Some(1),
            ..catalog_quest(P)
        };
        assert!(detect(&episode, vec![repeatable]).observations.is_empty());
    }

    #[test]
    fn quests_newly_in_the_giver_list_after_the_turn_in_are_list_handoffs() {
        let mut episode = turn_in_episode(vec![]);
        episode.offers = vec![gossip(90.0, G, &[300], &[P]), gossip(110.0, G, &[300, X, 201], &[])];
        let found = detect(&episode, vec![]);
        let list = HandoffKind::List { previous_list_at: 90.0 };
        assert_eq!(summary(&found.observations), [(list, P, X), (list, P, 201)]);
        assert!(found.observations.iter().all(|o| o.confounders.is_empty()));
        // 300 stayed available: judged (not unlocked by P), so it counts as compared.
        assert_eq!(found.compared_quests.into_iter().collect::<Vec<_>>(), [200, 201, 300]);
    }

    #[test]
    fn repeatable_quests_in_the_after_list_come_back_on_their_own() {
        let mut episode = turn_in_episode(vec![]);
        let mut after = gossip(110.0, G, &[X], &[]);
        after.available.push(OfferedQuest {
            id: 201,
            trivial: None,
            frequency: Some(2),
            repeatable: None,
        });
        episode.offers = vec![gossip(90.0, G, &[], &[]), after];
        let repeatable = CatalogQuest {
            special_flags: Some(1),
            ..catalog_quest(X)
        };
        assert!(detect(&episode, vec![repeatable]).observations.is_empty());
    }

    #[test]
    fn quests_hidden_before_because_they_were_in_the_log_are_skipped() {
        let mut episode = turn_in_episode(vec![]);
        episode.quest_log = log(&[(0.0, &[P, X]), (95.0, &[P]), (100.2, &[])]);
        episode.offers = vec![gossip(90.0, G, &[], &[]), gossip(110.0, G, &[X], &[])];
        assert_eq!(handoffs(&episode), []);
    }

    #[test]
    fn a_level_up_between_the_lists_confounds_unless_the_quest_was_already_level_eligible() {
        let mut episode = turn_in_episode(vec![]);
        episode.levels = vec![TimedValue { t: 0.0, v: 10 }, TimedValue { t: 105.0, v: 11 }];
        episode.offers = vec![gossip(90.0, G, &[], &[]), gossip(110.0, G, &[X, 201], &[])];
        let quests = vec![
            CatalogQuest {
                required_level: Some(11),
                ..catalog_quest(X)
            },
            CatalogQuest {
                required_level: Some(8),
                ..catalog_quest(201)
            },
        ];
        let confounders: Vec<_> = detect(&episode, quests).observations.into_iter().map(|o| (o.offered, o.confounders)).collect();
        assert_eq!(confounders, [(X, vec![Confounder::LevelUp { from: 10, to: 11 }]), (201, vec![])]);
    }

    #[test]
    fn other_completions_and_abandoned_quests_between_the_lists_confound() {
        let mut episode = turn_in_episode(vec![event(104.0, QuestEventKind::TurnedIn, 50, Some(npc(9)))]);
        episode.quest_log = log(&[(0.0, &[P, 50, 60]), (100.2, &[])]);
        episode.offers = vec![gossip(90.0, G, &[], &[]), gossip(110.0, G, &[X], &[])];
        assert_eq!(handoffs(&episode)[0].confounders, [Confounder::AlsoCompleted(50), Confounder::LeftLog(60)]);
    }

    #[test]
    fn resets_and_quests_entering_the_log_between_the_lists_confound() {
        let mut episode = turn_in_episode(vec![]);
        episode.completed = Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: vec![70],
            changes: vec![CompletedChange {
                t: 105.0,
                add: None,
                remove: Some(vec![70]),
            }],
        });
        episode.quest_log = log(&[(0.0, &[P]), (100.2, &[]), (105.0, &[80])]);
        episode.offers = vec![gossip(90.0, G, &[], &[]), gossip(110.0, G, &[X], &[])];
        assert_eq!(handoffs(&episode)[0].confounders, [Confounder::Reset(70), Confounder::EnteredLog(80)]);
    }
}
