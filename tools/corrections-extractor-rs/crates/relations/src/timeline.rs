//! Point-in-time lookups over one episode: "at time t, was quest X completed / in the log, and
//! what level was the character?". Every signal needs these, so they are built once per episode.

use crate::model::{CompletedTimeline, QuestEpisode, QuestEventKind, QuestId, SessionTime, TimedValue};
use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashSet};

/// Index of the last entry with `time <= t`, or None. `times` must be ascending.
fn last_at_or_before(times: &[f64], t: f64) -> Option<usize> {
    times.partition_point(|&time| time <= t).checked_sub(1)
}

#[derive(Clone, Copy)]
struct Toggle {
    t: SessionTime,
    completed: bool,
}

/// When a quest first became completed: already at capture start, or at a session time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Completion {
    Initial,
    At(SessionTime),
}

pub struct EpisodeTimeline<'e> {
    pub episode: &'e QuestEpisode,
    initial_completed: FxHashSet<QuestId>,
    /// Per quest, completion toggles sorted by time (stable: same-time toggles keep the order
    /// they were recorded in). Quests in the order their first toggle was met.
    toggles: IndexMap<QuestId, Vec<Toggle>, FxBuildHasher>,
    log_times: Vec<f64>,
    log_sets: Vec<FxHashSet<QuestId>>,
    level_times: Vec<f64>,
    level_values: Vec<i64>,
}

impl<'e> EpisodeTimeline<'e> {
    /// `include_turn_ins` treats a QUEST_TURNED_IN as completion from the event's time, before the
    /// (delay-sampled) completed delta shows it. Repeatable quests are never flagged completed by
    /// the client, so this over-reports them. The TS default is true.
    pub fn new(episode: &'e QuestEpisode, include_turn_ins: bool) -> Self {
        Self::with_completed(episode, episode.completed.as_ref(), include_turn_ins)
    }

    /// Like [`EpisodeTimeline::new`], with `completed` read in place of the episode's own completed
    /// history (e.g. one with catch-up reads folded in). `completed` is not kept.
    pub fn with_completed(episode: &'e QuestEpisode, completed: Option<&CompletedTimeline>, include_turn_ins: bool) -> Self {
        let initial_completed = completed.iter().flat_map(|c| c.initial.iter().copied()).collect();
        let mut toggles: IndexMap<QuestId, Vec<Toggle>, FxBuildHasher> = IndexMap::default();
        for change in completed.iter().flat_map(|c| &c.changes) {
            for &id in change.add.iter().flatten() {
                toggles.entry(id).or_default().push(Toggle { t: change.t, completed: true });
            }
            for &id in change.remove.iter().flatten() {
                toggles.entry(id).or_default().push(Toggle { t: change.t, completed: false });
            }
        }
        if include_turn_ins {
            for event in episode.quest_events.iter().filter(|e| e.kind == QuestEventKind::TurnedIn) {
                toggles.entry(event.quest_id).or_default().push(Toggle { t: event.t, completed: true });
            }
        }
        for list in toggles.values_mut() {
            list.sort_by(|a, b| a.t.total_cmp(&b.t));
        }
        EpisodeTimeline {
            episode,
            initial_completed,
            toggles,
            log_times: episode.quest_log.iter().map(|entry| entry.t).collect(),
            log_sets: episode.quest_log.iter().map(|entry| entry.v.iter().copied().collect()).collect(),
            level_times: episode.levels.iter().map(|entry| entry.t).collect(),
            level_values: episode.levels.iter().map(|entry| entry.v).collect(),
        }
    }

    pub fn is_completed_at(&self, quest_id: QuestId, t: SessionTime) -> bool {
        let Some(list) = self.toggles.get(&quest_id) else {
            return self.initial_completed.contains(&quest_id);
        };
        match list.partition_point(|toggle| toggle.t <= t).checked_sub(1) {
            Some(index) => list[index].completed,
            None => self.initial_completed.contains(&quest_id),
        }
    }

    /// The full completed set at `t`. Costs O(completed history); prefer `is_completed_at` in loops.
    pub fn completed_at(&self, t: SessionTime) -> FxHashSet<QuestId> {
        let mut result = self.initial_completed.clone();
        for &quest_id in self.toggles.keys() {
            if self.is_completed_at(quest_id, t) {
                result.insert(quest_id);
            } else {
                result.remove(&quest_id);
            }
        }
        result
    }

    /// When the quest first became completed in this session, if ever.
    pub fn completion_time(&self, quest_id: QuestId) -> Option<Completion> {
        if self.initial_completed.contains(&quest_id) {
            return Some(Completion::Initial);
        }
        let first = self.toggles.get(&quest_id)?.iter().find(|toggle| toggle.completed)?;
        Some(Completion::At(first.t))
    }

    /// Quests that became completed during the session, with their first completion time, in
    /// time order (same-time completions in the order their quests were first met).
    pub fn completions_during_session(&self) -> Vec<TimedValue<QuestId>> {
        let mut result: Vec<TimedValue<QuestId>> = self
            .toggles
            .iter()
            .filter(|(id, _)| !self.initial_completed.contains(id))
            .filter_map(|(&id, list)| list.iter().find(|toggle| toggle.completed).map(|first| TimedValue { t: first.t, v: id }))
            .collect();
        result.sort_by(|a, b| a.t.total_cmp(&b.t));
        result
    }

    /// The quest log at `t`; empty before the first entry.
    pub fn log_at(&self, t: SessionTime) -> Option<&FxHashSet<QuestId>> {
        last_at_or_before(&self.log_times, t).map(|index| &self.log_sets[index])
    }

    pub fn is_in_log_at(&self, quest_id: QuestId, t: SessionTime) -> bool {
        self.log_at(t).is_some_and(|log| log.contains(&quest_id))
    }

    /// Level at `t`; before the first change point, the first recorded level (the level at
    /// capture start).
    pub fn level_at(&self, t: SessionTime) -> Option<i64> {
        match last_at_or_before(&self.level_times, t) {
            Some(index) => Some(self.level_values[index]),
            None => self.level_values.first().copied(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CompletedChange, CompletedSource, CompletedTimeline};
    use crate::test_support::{event, make_episode};

    fn episode_with_history() -> QuestEpisode {
        let mut episode = make_episode();
        episode.completed = Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: vec![1, 2],
            changes: vec![
                CompletedChange {
                    t: 10.0,
                    add: Some(vec![3]),
                    remove: Some(vec![2]),
                },
                CompletedChange {
                    t: 20.0,
                    add: Some(vec![2]),
                    remove: None,
                },
            ],
        });
        episode
    }

    #[test]
    fn completion_follows_the_initial_set_and_its_changes_over_time() {
        let episode = episode_with_history();
        let timeline = EpisodeTimeline::new(&episode, true);
        assert!(timeline.is_completed_at(2, 5.0));
        assert!(!timeline.is_completed_at(2, 15.0), "a daily reset removed it");
        assert!(timeline.is_completed_at(2, 20.0));
        assert!(!timeline.is_completed_at(3, 9.9));
        assert!(timeline.is_completed_at(3, 10.0));
        let mut at_15: Vec<_> = timeline.completed_at(15.0).into_iter().collect();
        at_15.sort();
        assert_eq!(at_15, [1, 3]);
    }

    #[test]
    fn a_turn_in_counts_as_completion_before_the_delayed_delta_unless_disabled() {
        let mut episode = make_episode();
        episode.quest_events = vec![event(5.0, QuestEventKind::TurnedIn, 7, None)];
        assert!(EpisodeTimeline::new(&episode, true).is_completed_at(7, 5.0));
        assert!(!EpisodeTimeline::new(&episode, false).is_completed_at(7, 5.0));
    }

    #[test]
    fn completion_times_tell_initial_from_in_session_completions() {
        let episode = episode_with_history();
        let timeline = EpisodeTimeline::new(&episode, true);
        assert_eq!(timeline.completion_time(1), Some(Completion::Initial));
        assert_eq!(timeline.completion_time(3), Some(Completion::At(10.0)));
        assert_eq!(timeline.completion_time(4), None);
        let during: Vec<_> = timeline.completions_during_session().into_iter().map(|c| (c.t, c.v)).collect();
        assert_eq!(during, [(10.0, 3)], "2 was completed at the start, so its re-completion is no new completion");
    }

    #[test]
    fn log_and_level_are_read_at_the_last_change_point() {
        let mut episode = make_episode();
        episode.quest_log = vec![TimedValue { t: 0.0, v: vec![] }, TimedValue { t: 4.0, v: vec![9] }];
        episode.levels = vec![TimedValue { t: 0.0, v: 10 }, TimedValue { t: 50.0, v: 11 }];
        let timeline = EpisodeTimeline::new(&episode, true);
        assert!(!timeline.is_in_log_at(9, 3.9));
        assert!(timeline.is_in_log_at(9, 4.0));
        assert_eq!(timeline.level_at(-1.0), Some(10), "before the first change point: the first level");
        assert_eq!(timeline.level_at(49.0), Some(10));
        assert_eq!(timeline.level_at(50.0), Some(11));
    }
}
