//! Character state at a moment of one episode, tolerant of the addon's sampling lag.
//!
//! The quest log and completed set are sampled up to ~1s after the event that changed them
//! (QuestLog.lua / CompletedQuests.lua SAMPLE_DELAYS), while QUEST_ACCEPTED / QUEST_TURNED_IN
//! are recorded when they fire. A giver list re-read right after accepting Y can therefore
//! predate the log sample that contains Y. [`EpisodeStates::in_log`] and [`EpisodeStates::log`]
//! patch the sampled log with the quest events of the last `slack` seconds so both sides of a
//! comparison see the same truth.
//!
//! Some completed-set deltas are catch-up reads that add hundreds of old completions at once.
//! Their timestamps are meaningless: those quests count as completed from the session start.
//!
//! Sets come back in the order the TypeScript pipeline lists them (a JS Set keeps insertion
//! order): that order is the order of a blocked moment's suspects, and through them of the
//! floating-point sums over suspects and of the order in which explanations are first named.

use crate::model::{CompletedChange, CompletedTimeline, QuestEpisode, QuestEvent, QuestEventKind, QuestId, SessionTime};
use crate::timeline::EpisodeTimeline;
use indexmap::{IndexMap, IndexSet};
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use std::borrow::Cow;
use std::ops::Deref;
use std::rc::Rc;

/// Quest ids in insertion order.
pub type QuestSet = IndexSet<QuestId, FxBuildHasher>;

/// An episode with its catch-up deltas folded into the initial completed set: what
/// [`EpisodeStates`] reads. Made by [`without_catch_ups`].
pub struct CaughtUp<'e>(Cow<'e, QuestEpisode>);

impl Deref for CaughtUp<'_> {
    type Target = QuestEpisode;

    fn deref(&self) -> &QuestEpisode {
        &self.0
    }
}

pub struct EpisodeStates<'e> {
    /// The episode with catch-up deltas folded into the initial completed set.
    pub episode: &'e QuestEpisode,
    pub timeline: EpisodeTimeline<'e>,
    slack: f64,
    /// Quest events sorted by time (stable).
    events: Vec<&'e QuestEvent>,
    event_times: Vec<f64>,
    /// Ascending times at which the completed set may change; the count at or before t versions it.
    completion_change_times: Vec<f64>,
    /// Every quest the completed set can hold: the initial set, then the quests the completed-set
    /// changes and turn-ins name, in the order they are first named.
    completable: Vec<QuestId>,
    /// Keyed by (completion version, quests turned in within the slack): equal keys, equal sets.
    completed_by_key: FxHashMap<(usize, Vec<QuestId>), Rc<QuestSet>>,
}

impl<'e> EpisodeStates<'e> {
    pub fn new(episode: &'e CaughtUp<'_>, slack: f64) -> Self {
        let episode: &'e QuestEpisode = episode;
        let mut events: Vec<&QuestEvent> = episode.quest_events.iter().collect();
        events.sort_by(|a, b| a.t.total_cmp(&b.t));
        let changes = || episode.completed.iter().flat_map(|completed| &completed.changes);
        let turn_ins = || episode.quest_events.iter().filter(|event| event.kind == QuestEventKind::TurnedIn);

        let mut completion_change_times: Vec<f64> = changes().map(|change| change.t).chain(turn_ins().map(|event| event.t)).collect();
        completion_change_times.sort_by(f64::total_cmp);

        let mut completable: QuestSet = episode.completed.iter().flat_map(|completed| completed.initial.iter().copied()).collect();
        for change in changes() {
            completable.extend(change.add.iter().flatten().copied());
            completable.extend(change.remove.iter().flatten().copied());
        }
        completable.extend(turn_ins().map(|event| event.quest_id));

        EpisodeStates {
            episode,
            timeline: EpisodeTimeline::new(episode, true),
            slack,
            event_times: events.iter().map(|event| event.t).collect(),
            events,
            completion_change_times,
            completable: completable.into_iter().collect(),
            completed_by_key: FxHashMap::default(),
        }
    }

    pub fn is_completed(&mut self, quest_id: QuestId, t: SessionTime) -> bool {
        self.completed(t).contains(&quest_id)
    }

    /// The completed set at t, allowing for sampling lag: quests the completed-set delta reports
    /// within `slack` after t count as already completed, unless a turn-in event shows they were
    /// completed after t. Quests completed as a side effect (Forever completes every zone copy of
    /// a quest together) only ever arrive through the delayed delta.
    ///
    /// Two reads with no completion change between them return the same `Rc`, so callers can
    /// tell "nothing changed" with `Rc::ptr_eq`.
    pub fn completed(&mut self, t: SessionTime) -> Rc<QuestSet> {
        let turned_in = self.turn_ins_within(t);
        let key = (count_at_or_before(&self.completion_change_times, t + self.slack), turned_in);
        if let Some(set) = self.completed_by_key.get(&key) {
            return Rc::clone(set);
        }
        let at = t + self.slack;
        let set: QuestSet = self
            .completable
            .iter()
            .copied()
            .filter(|&quest_id| self.timeline.is_completed_at(quest_id, at) && !key.1.contains(&quest_id))
            .collect();
        let set = Rc::new(set);
        self.completed_by_key.insert(key, Rc::clone(&set));
        set
    }

    /// Quests turned in during (t, t + slack].
    fn turn_ins_within(&self, t: SessionTime) -> Vec<QuestId> {
        self.events[count_at_or_before(&self.event_times, t)..]
            .iter()
            .take_while(|event| event.t <= t + self.slack)
            .filter(|event| event.kind == QuestEventKind::TurnedIn)
            .map(|event| event.quest_id)
            .collect()
    }

    /// The quest log at t, corrected by accept/leave events of the last `slack` seconds.
    pub fn log(&self, t: SessionTime) -> QuestSet {
        // The timeline's log sets are unordered; the sampled list keeps the order suspects need.
        let sampled = self.episode.quest_log.partition_point(|entry| entry.t <= t).checked_sub(1);
        let mut result: QuestSet = sampled
            .map(|index| self.episode.quest_log[index].v.iter().copied().collect())
            .unwrap_or_default();
        for (quest_id, in_log) in self.recent_log_events(t) {
            if in_log {
                result.insert(quest_id);
            } else {
                result.shift_remove(&quest_id);
            }
        }
        result
    }

    pub fn in_log(&self, quest_id: QuestId, t: SessionTime) -> bool {
        match self.recent_log_events(t).get(&quest_id) {
            Some(&in_log) => in_log,
            None => self.timeline.is_in_log_at(quest_id, t),
        }
    }

    /// Any quest event at all within (from, to].
    pub fn any_event_within(&self, from: SessionTime, to: SessionTime) -> bool {
        count_at_or_before(&self.event_times, to) > count_at_or_before(&self.event_times, from)
    }

    /// Any quest event (dialog, accept, turn-in, ...) for the quest within [from, to].
    pub fn touched(&self, quest_id: QuestId, from: SessionTime, to: SessionTime) -> bool {
        self.events[count_at_or_before(&self.event_times, from - 1e-9)..]
            .iter()
            .take_while(|event| event.t <= to)
            .any(|event| event.quest_id == quest_id)
    }

    /// Per quest, whether its last accept/leave event in (t - slack, t] put it in the log; quests
    /// in the order of their first such event.
    fn recent_log_events(&self, t: SessionTime) -> IndexMap<QuestId, bool, FxBuildHasher> {
        let mut result = IndexMap::default();
        let recent = count_at_or_before(&self.event_times, t - self.slack)..count_at_or_before(&self.event_times, t);
        for event in &self.events[recent] {
            match event.kind {
                QuestEventKind::Accepted => {
                    result.insert(event.quest_id, true);
                }
                QuestEventKind::TurnedIn | QuestEventKind::Abandoned | QuestEventKind::Removed => {
                    result.insert(event.quest_id, false);
                }
                QuestEventKind::Detail | QuestEventKind::Progress | QuestEventKind::Complete => {}
            }
        }
        result
    }
}

/// Moves the quests of catch-up deltas (more than `limit` additions) into the initial completed
/// set, except quests with their own events this session, whose completion time the events tell.
pub fn without_catch_ups(episode: &QuestEpisode, limit: usize) -> CaughtUp<'_> {
    let is_catch_up = |change: &CompletedChange| change.add.as_ref().is_some_and(|add| add.len() > limit);
    let Some(completed) = episode.completed.as_ref().filter(|completed| completed.changes.iter().any(is_catch_up)) else {
        return CaughtUp(Cow::Borrowed(episode));
    };
    let active: FxHashSet<QuestId> = episode.quest_events.iter().map(|event| event.quest_id).collect();
    let mut initial: QuestSet = completed.initial.iter().copied().collect();
    let mut changes = Vec::with_capacity(completed.changes.len());
    for change in &completed.changes {
        match &change.add {
            Some(add) if is_catch_up(change) => {
                initial.extend(add.iter().copied().filter(|quest_id| !active.contains(quest_id)));
                let add: Vec<QuestId> = add.iter().copied().filter(|quest_id| active.contains(quest_id)).collect();
                if !add.is_empty() || change.remove.is_some() {
                    changes.push(CompletedChange {
                        add: Some(add),
                        ..change.clone()
                    });
                }
            }
            _ => changes.push(change.clone()),
        }
    }
    let completed = CompletedTimeline {
        source: completed.source,
        initial: initial.into_iter().collect(),
        changes,
    };
    CaughtUp(Cow::Owned(QuestEpisode {
        completed: Some(completed),
        ..episode.clone()
    }))
}

/// Number of entries `<= t` in an ascending slice.
fn count_at_or_before(times: &[f64], t: f64) -> usize {
    times.partition_point(|&time| time <= t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CompletedSource, TimedValue};
    use crate::test_support::{event, make_episode, npc};

    fn modern(initial: Vec<QuestId>, changes: Vec<CompletedChange>) -> Option<CompletedTimeline> {
        Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial,
            changes,
        })
    }

    fn added(t: f64, add: Vec<QuestId>) -> CompletedChange {
        CompletedChange {
            t,
            add: Some(add),
            remove: None,
        }
    }

    fn episode() -> QuestEpisode {
        QuestEpisode {
            // 11 is turned in at 50.0; Forever also completes its zone copy 12, which only the
            // delayed completed-set delta reports.
            completed: modern(vec![1], vec![added(50.8, vec![11, 12])]),
            quest_log: vec![
                TimedValue { t: 0.0, v: vec![11] },
                TimedValue { t: 30.9, v: vec![11, 21] },
                TimedValue { t: 50.9, v: vec![] },
            ],
            quest_events: vec![
                event(30.0, QuestEventKind::Accepted, 21, Some(npc(5))),
                event(50.0, QuestEventKind::TurnedIn, 11, Some(npc(5))),
                event(70.0, QuestEventKind::Abandoned, 21, None),
            ],
            ..make_episode()
        }
    }

    #[test]
    fn counts_completions_the_delayed_delta_reports_within_the_slack_except_later_turn_ins() {
        let episode = episode();
        let caught_up = without_catch_ups(&episode, 60);
        let mut states = EpisodeStates::new(&caught_up, 2.0);
        assert!(states.is_completed(12, 50.1));
        assert!(states.is_completed(11, 50.1));
        // Read just before the turn-in: 11 is still in progress even though the delta follows soon.
        assert!(!states.is_completed(11, 49.5));
    }

    #[test]
    fn treats_a_catch_up_read_as_completed_from_the_start_except_quests_active_this_session() {
        let mut add: Vec<QuestId> = (1000..1070).collect();
        add.push(77);
        let catch_up = QuestEpisode {
            completed: modern(vec![], vec![added(400.0, add)]),
            quest_events: vec![event(399.0, QuestEventKind::TurnedIn, 77, Some(npc(5)))],
            ..make_episode()
        };
        let caught_up = without_catch_ups(&catch_up, 60);
        let mut states = EpisodeStates::new(&caught_up, 2.0);
        assert!(states.is_completed(1000, 5.0));
        assert!(!states.is_completed(77, 5.0));
        assert!(states.is_completed(77, 399.0));
    }

    #[test]
    fn patches_the_sampled_log_with_accepts_and_removals_the_sample_has_not_caught_up_with() {
        let episode = episode();
        let caught_up = without_catch_ups(&episode, 60);
        let states = EpisodeStates::new(&caught_up, 2.0);
        assert!(states.in_log(21, 30.2));
        assert!(states.log(30.2).contains(&21));
        assert!(!states.in_log(21, 29.0));
        // Turned in at 50.0, still in the sample taken at 30.9.
        assert!(!states.in_log(11, 50.5));
        assert!(!states.log(50.5).contains(&11));
    }

    #[test]
    fn lists_the_completed_set_in_the_order_the_typescript_sets_do() {
        // The recorded initial order, then quests in the order the changes and turn-ins first name
        // them: the order of a blocked moment's suspects, and so of the sums over them.
        let episode = QuestEpisode {
            completed: modern(vec![5, 1, 5], vec![added(10.0, vec![9, 3])]),
            quest_events: vec![event(20.0, QuestEventKind::TurnedIn, 2, Some(npc(5)))],
            ..make_episode()
        };
        let caught_up = without_catch_ups(&episode, 60);
        let mut states = EpisodeStates::new(&caught_up, 2.0);
        assert_eq!(states.completed(30.0).iter().copied().collect::<Vec<_>>(), [5, 1, 9, 3, 2]);
    }
}
