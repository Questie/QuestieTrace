//! One merged completion history per character: which quests it completed and took, and where in
//! its (sessions-ordered) life each one first happened. Everything this signal infers is a
//! statement about these histories across characters.
//!
//! Order is only partly observable. A quest already completed (or already in the log) when a
//! session started happened at an unknown time since the previous session, so two such quests of
//! the same session are unordered against each other, but both precede everything that happens
//! during that session and follow everything from earlier sessions.

use crate::model::{CompletedTimeline, PlayerContext, QuestCatalog, QuestEpisode, QuestEventKind, QuestId, SessionTime};
use crate::timeline::{Completion, EpisodeTimeline};
use indexmap::IndexMap;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::hash_map::Entry;

/// Quests first seen in the log this early in a session, without an accept, were held at its start.
const LATE_LOG_SECONDS: SessionTime = 60.0;

/// Lag allowed between two exact event times before their order counts (event timestamps jitter).
const EXACT_TOLERANCE_SECONDS: SessionTime = 1.0;

/// When, within its session, a fact became true.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum When {
    /// Already true when the session started: it happened at an unknown time since the previous
    /// session.
    AtStart,
    /// At session time `t`. `exact`: `t` is the event's own time (a turn-in or accept), not a later
    /// sample of the completed set or log.
    During { t: SessionTime, exact: bool },
}

/// Where one fact sits in a character's history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp {
    /// The session it was first seen in: an index into [`CharacterHistory::sessions`].
    pub session: usize,
    pub when: When,
    /// Completions only: when a completed-set sample first added the quest in this session, if it
    /// did. This is an observation time, not the server's exact flag time; a turn-in may date
    /// `when` earlier.
    pub flagged: Option<SessionTime>,
}

impl Stamp {
    /// History order: sessions in order, and within a session what was already true at its start
    /// before everything that happened during it. (The TS pipeline packs this into one number,
    /// `session * 1e7 + t`, with "at start" just below the session; for session times >= 0, which
    /// is all ingest writes, both orders agree.)
    pub fn cmp_position(&self, other: &Stamp) -> Ordering {
        let time = |stamp: &Stamp| match stamp.when {
            When::AtStart => SessionTime::NEG_INFINITY,
            When::During { t, .. } => t,
        };
        self.session.cmp(&other.session).then_with(|| time(self).total_cmp(&time(other)))
    }

    pub fn is_at_start(&self) -> bool {
        self.when == When::AtStart
    }

    /// The session time, unless it was already true at the session's start.
    pub fn time(&self) -> Option<SessionTime> {
        match self.when {
            When::AtStart => None,
            When::During { t, .. } => Some(t),
        }
    }

    /// When the completed set showed it (falling back to the event time), unless it was already
    /// true at the session's start. Co-flagged siblings only ever show up in the completed set.
    pub fn flagged_time(&self) -> Option<SessionTime> {
        self.time().map(|t| self.flagged.unwrap_or(t))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Before,
    After,
    Unordered,
}

/// Whether completion `done` happened before or after `taken`. Completion times are never early
/// (a turn-in is exact, a completed-quest delta is sampled up to ~1 s late plus server lag), so any
/// in-session lead counts as "before" - a follow-up accepted a second after its prerequisite's
/// turn-in is the most common case. A sampled completion up to `tolerance_seconds` late is
/// "unordered", not "after": "accepted the follow-up 0.5 s before the prerequisite's delta arrived"
/// proves nothing. A turn-in after the accept does: the server would not have offered X yet.
pub fn order(done: &Stamp, taken: &Stamp, tolerance_seconds: SessionTime) -> Order {
    match done.cmp_position(taken) {
        Ordering::Equal => Order::Unordered,
        Ordering::Less => Order::Before,
        Ordering::Greater => match (done.when, taken.when) {
            (When::During { t: done_t, exact }, When::During { t: taken_t, .. }) if done.session == taken.session => {
                let tolerance = if exact { EXACT_TOLERANCE_SECONDS } else { tolerance_seconds };
                if done_t - taken_t <= tolerance {
                    Order::Unordered
                } else {
                    Order::After
                }
            }
            _ => Order::After,
        },
    }
}

/// Seconds from `earlier` to `later` when both happened during the same session (neither was
/// already true when it started).
pub fn lead_within_session(earlier: &Stamp, later: &Stamp) -> Option<SessionTime> {
    match (earlier.time(), later.time()) {
        (Some(from), Some(to)) if earlier.session == later.session => Some(to - from),
        _ => None,
    }
}

pub struct CharacterHistory<'e> {
    pub key: &'e str,
    pub player: PlayerContext,
    /// Highest level seen in any session; a lower bound on what the character reached.
    pub max_level: Option<i64>,
    /// One timeline per kept session, in history order, for point-in-time checks (levels, log,
    /// completion). A [`Stamp`]'s `session` indexes this.
    pub sessions: Vec<EpisodeTimeline<'e>>,
    /// First completion of each quest. Only quests the client's completed set actually contained.
    pub done: FxHashMap<QuestId, Stamp>,
    /// First moment each quest was in the log, accepted, or completed.
    pub taken: FxHashMap<QuestId, Stamp>,
}

impl CharacterHistory<'_> {
    /// The episode a stamp was seen in, for evidence.
    pub fn episode_key(&self, stamp: &Stamp) -> &str {
        &self.sessions[stamp.session].episode.key
    }
}

/// The character had both quests active at once: both in the log, or one in the log while the
/// other was completed. Exclusive quests never are.
pub fn held_together(history: &CharacterHistory, a: QuestId, b: QuestId) -> bool {
    history.sessions.iter().any(|timeline| {
        timeline.episode.quest_log.iter().any(|entry| {
            let has_a = entry.v.contains(&a);
            let has_b = entry.v.contains(&b);
            (has_a && (has_b || timeline.is_completed_at(b, entry.t))) || (has_b && timeline.is_completed_at(a, entry.t))
        })
    })
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryReport {
    pub episodes: usize,
    /// Sessions dropped because they carry no completed-quest data, or an empty set that cannot be
    /// real.
    pub without_completed: usize,
    /// Sessions dropped because their initial completed set lost much of what earlier sessions
    /// showed.
    pub broken_initial_sets: usize,
    pub characters: usize,
    /// Characters dropped because their sessions disagree on race or class (a grouping error).
    pub inconsistent_characters: usize,
}

/// Quests whose completion state is not a one-way "done forever" flag, so co-occurrence says
/// nothing about them.
pub fn unstable_quests(episodes: &[&QuestEpisode], catalog: &QuestCatalog) -> FxHashSet<QuestId> {
    let mut unstable: FxHashSet<QuestId> = catalog
        .quests
        .values()
        .filter(|quest| is_repeatable(quest.special_flags))
        .map(|quest| quest.id)
        .collect();
    for episode in episodes {
        if let Some(completed) = &episode.completed {
            // Daily/weekly resets remove a few quests from the completed set. A change that drops
            // most of the set is a bad read (seen mid-session, followed by the set coming back), not
            // a reset.
            let mut size = completed.initial.len() as i64;
            for change in &completed.changes {
                let removed = change.remove.as_deref().unwrap_or_default();
                let added = change.add.as_deref().unwrap_or_default();
                let removed_count = removed.len() as i64;
                if removed_count <= 2 || 2 * removed_count <= size {
                    unstable.extend(removed);
                }
                size += added.len() as i64 - removed_count;
            }
        }
        for offer in &episode.offers {
            for quest in &offer.available {
                if quest.repeatable == Some(true) || quest.frequency.unwrap_or(1) > 1 {
                    unstable.insert(quest.id);
                }
            }
        }
    }
    unstable
}

/// specialFlags bit 1 (repeatable).
fn is_repeatable(special_flags: Option<i64>) -> bool {
    special_flags.unwrap_or(0) % 2 == 1
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryParams {
    /// A session whose initial completed set misses more than this share of the stable quests
    /// earlier sessions already showed is treated as a broken read and dropped.
    pub max_initial_loss: f64,
}

/// A session kept for its character's history, with the completed set that qualified it.
struct Session<'e> {
    episode: &'e QuestEpisode,
    completed: &'e CompletedTimeline,
}

/// Every character's history, in the order characters first appear in `episodes`. Quests in
/// `ignore` (see [`unstable_quests`]) are left out of `done`/`taken` entirely.
pub fn build_histories<'e>(episodes: &[&'e QuestEpisode], params: &HistoryParams, ignore: &FxHashSet<QuestId>) -> (Vec<CharacterHistory<'e>>, HistoryReport) {
    let mut report = HistoryReport {
        episodes: episodes.len(),
        ..HistoryReport::default()
    };

    let mut by_character: IndexMap<&'e str, Vec<Session<'e>>, FxBuildHasher> = IndexMap::default();
    for &episode in episodes {
        match episode.completed.as_ref().filter(|completed| !completed_looks_missing(episode, completed)) {
            Some(completed) => by_character.entry(&episode.character_key).or_default().push(Session { episode, completed }),
            None => report.without_completed += 1,
        }
    }

    // Characters are independent, so they are built side by side (in order).
    let characters: Vec<(&'e str, Vec<Session<'e>>)> = by_character.into_iter().collect();
    let built: Vec<Option<Built<'e>>> = characters
        .into_par_iter()
        .map(|(key, mut sessions)| {
            let player = merge_player(&sessions)?;
            // Ingest orders a character's sessions by completed-history size, then name (a third
            // have no name), so every signal agrees on what happened first. sessionOrder is unique
            // per character; the key only makes the order total.
            sessions.sort_by(|a, b| {
                a.episode
                    .session_order
                    .cmp(&b.episode.session_order)
                    .then_with(|| a.episode.key.cmp(&b.episode.key))
            });
            Some(build_history(key, player, &sessions, params, ignore))
        })
        .collect();

    let mut histories = Vec::new();
    for built in built {
        let Some(Built { history, broken_initial_sets }) = built else {
            report.inconsistent_characters += 1;
            continue;
        };
        report.broken_initial_sets += broken_initial_sets;
        if !history.done.is_empty() || !history.taken.is_empty() {
            histories.push(history);
        }
    }
    report.characters = histories.len();
    (histories, report)
}

/// One character's history, and how many of its sessions were dropped as broken reads.
struct Built<'e> {
    history: CharacterHistory<'e>,
    broken_initial_sets: usize,
}

/// The character's race/class/faction, or None when its sessions disagree.
fn merge_player(sessions: &[Session]) -> Option<PlayerContext> {
    fn merge<T: Copy + PartialEq>(merged: &mut Option<T>, value: Option<T>) -> bool {
        match (*merged, value) {
            (Some(known), Some(value)) => known == value,
            (None, Some(value)) => {
                *merged = Some(value);
                true
            }
            (_, None) => true,
        }
    }
    let mut merged = PlayerContext::default();
    for Session { episode, .. } in sessions {
        let player = &episode.player;
        let consistent =
            merge(&mut merged.race_id, player.race_id) && merge(&mut merged.class_id, player.class_id) && merge(&mut merged.faction, player.faction);
        if !consistent {
            return None;
        }
    }
    Some(merged)
}

/// Records `stamp` as the quest's first moment unless an earlier one is known.
fn keep_first(map: &mut FxHashMap<QuestId, Stamp>, ignore: &FxHashSet<QuestId>, quest_id: QuestId, stamp: Stamp) {
    if ignore.contains(&quest_id) {
        return;
    }
    match map.entry(quest_id) {
        Entry::Occupied(mut existing) => {
            if stamp.cmp_position(existing.get()) == Ordering::Less {
                existing.insert(stamp);
            }
        }
        Entry::Vacant(slot) => {
            slot.insert(stamp);
        }
    }
}

fn build_history<'e>(key: &'e str, player: PlayerContext, sessions: &[Session<'e>], params: &HistoryParams, ignore: &FxHashSet<QuestId>) -> Built<'e> {
    let mut history = CharacterHistory {
        key,
        player,
        max_level: None,
        sessions: Vec::new(),
        done: FxHashMap::default(),
        taken: FxHashMap::default(),
    };
    let mut broken_initial_sets = 0;

    for &Session { episode, completed } in sessions {
        if initial_set_looks_broken(&completed.initial, &history.done, params.max_initial_loss) {
            broken_initial_sets += 1;
            continue;
        }
        let session = history.sessions.len();
        let at_start = Stamp {
            session,
            when: When::AtStart,
            flagged: None,
        };
        let during = |t: SessionTime, exact: bool| Stamp {
            session,
            when: When::During { t, exact },
            flagged: None,
        };
        let accepted: FxHashSet<QuestId> = episode
            .quest_events
            .iter()
            .filter(|event| event.kind == QuestEventKind::Accepted)
            .map(|event| event.quest_id)
            .collect();
        let turned_in_at = |quest_id: QuestId, t: SessionTime| {
            episode
                .quest_events
                .iter()
                .any(|event| event.kind == QuestEventKind::TurnedIn && event.quest_id == quest_id && event.t == t)
        };

        // Turn-ins date a completion more precisely than the delayed delta, but only quests the
        // completed set really contained count as done (repeatables are turned in, never flagged).
        let timeline = EpisodeTimeline::new(episode, true);
        let mut flagged_at: FxHashMap<QuestId, SessionTime> = FxHashMap::default();
        let mut added = Vec::new();
        for change in &completed.changes {
            for &id in change.add.iter().flatten() {
                if let Entry::Vacant(slot) = flagged_at.entry(id) {
                    slot.insert(change.t);
                    added.push(id);
                }
            }
        }
        for &quest_id in completed.initial.iter().chain(&added) {
            let stamp = match timeline.completion_time(quest_id) {
                None => continue,
                Some(Completion::Initial) => at_start,
                Some(Completion::At(t)) => Stamp {
                    flagged: flagged_at.get(&quest_id).copied(),
                    ..during(t, turned_in_at(quest_id, t))
                },
            };
            keep_first(&mut history.done, ignore, quest_id, stamp);
            keep_first(&mut history.taken, ignore, quest_id, stamp);
        }

        // The log often loads a few seconds into the session (5% of sessions start with an empty
        // snapshot): quests that show up early without being accepted were already held.
        for (entry_index, entry) in episode.quest_log.iter().enumerate() {
            for &quest_id in &entry.v {
                let loaded_late = entry.t <= LATE_LOG_SECONDS && !accepted.contains(&quest_id);
                let stamp = if entry_index == 0 || loaded_late { at_start } else { during(entry.t, false) };
                keep_first(&mut history.taken, ignore, quest_id, stamp);
            }
        }
        for event in episode.quest_events.iter().filter(|event| event.kind == QuestEventKind::Accepted) {
            keep_first(&mut history.taken, ignore, event.quest_id, during(event.t, true));
        }

        for level in &episode.levels {
            history.max_level = Some(history.max_level.map_or(level.v, |max| max.max(level.v)));
        }
        history.sessions.push(timeline);
    }
    Built { history, broken_initial_sets }
}

/// Old traces using the legacy GetQuestsCompleted often recorded an empty set for the whole
/// session. Such a session would make every quest the character held look taken without its
/// prerequisites. An empty set is believable only for a fresh character whose turn-ins all show
/// up as additions.
fn completed_looks_missing(episode: &QuestEpisode, completed: &CompletedTimeline) -> bool {
    if !completed.initial.is_empty() {
        return false;
    }
    if episode.levels.first().map_or(1, |level| level.v) > 5 {
        return true;
    }
    let added: FxHashSet<QuestId> = completed.changes.iter().flat_map(|change| change.add.iter().flatten().copied()).collect();
    episode
        .quest_events
        .iter()
        .any(|event| event.kind == QuestEventKind::TurnedIn && !added.contains(&event.quest_id))
}

/// An initial completed set that forgot many quests earlier sessions had completed is a bad read.
fn initial_set_looks_broken(initial: &[QuestId], known: &FxHashMap<QuestId, Stamp>, max_loss: f64) -> bool {
    if known.is_empty() {
        return false;
    }
    let present: FxHashSet<QuestId> = initial.iter().copied().collect();
    let missing = known.keys().filter(|quest_id| !present.contains(quest_id)).count();
    missing as f64 > (known.len() as f64 * max_loss).max(2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CompletedChange, CompletedSource, TimedValue};
    use crate::signals::cooccurrence::fixtures::{catalog_of, session, SessionSpec};
    use crate::test_support::{event, make_episode};

    const PARAMS: HistoryParams = HistoryParams { max_initial_loss: 0.1 };

    fn at(t: SessionTime, exact: bool) -> Stamp {
        Stamp {
            session: 0,
            when: When::During { t, exact },
            flagged: None,
        }
    }

    #[test]
    fn any_lead_of_a_completion_is_before_but_a_short_lag_of_a_sampled_one_is_unordered() {
        // Completed-quest deltas arrive late, never early.
        assert_eq!(order(&at(100.0, false), &at(100.5, false), 10.0), Order::Before);
        assert_eq!(order(&at(101.0, false), &at(100.0, false), 10.0), Order::Unordered);
        assert_eq!(order(&at(130.0, false), &at(100.0, false), 10.0), Order::After);
        // A turn-in two seconds after accepting the follow-up is no sampling delay.
        assert_eq!(order(&at(102.0, true), &at(100.0, true), 10.0), Order::After);
    }

    #[test]
    fn a_characters_sessions_are_ordered_by_session_order_not_input_order() {
        let later = session(
            "c",
            SessionSpec {
                order: 1,
                initial: vec![1, 2],
                turn_ins: vec![(50.0, 3)],
                ..SessionSpec::default()
            },
        );
        let earlier = session(
            "c",
            SessionSpec {
                initial: vec![1],
                accepts: vec![(10.0, 2)],
                turn_ins: vec![(20.0, 2)],
                ..SessionSpec::default()
            },
        );
        let (histories, _) = build_histories(&[&later, &earlier], &PARAMS, &FxHashSet::default());
        let history = &histories[0];

        let keys: Vec<&str> = history.sessions.iter().map(|timeline| timeline.episode.key.as_str()).collect();
        assert_eq!(keys, [earlier.key.as_str(), later.key.as_str()]);
        assert_eq!(order(&history.done[&2], &history.taken[&3], 10.0), Order::Before);
    }

    #[test]
    fn quests_appearing_in_a_late_loading_log_are_dated_to_the_session_start() {
        // The log is empty until t=5; quest 3 was never accepted this session, so it was already held.
        let mut late_log = make_episode();
        late_log.character_key = "c".into();
        late_log.completed = Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: vec![1],
            changes: vec![CompletedChange {
                t: 2.5,
                add: Some(vec![2]),
                remove: None,
            }],
        });
        late_log.quest_log = vec![TimedValue { t: 0.0, v: vec![] }, TimedValue { t: 5.0, v: vec![3] }];
        late_log.quest_events = vec![event(2.0, QuestEventKind::TurnedIn, 2, None)];
        let (histories, _) = build_histories(&[&late_log], &PARAMS, &FxHashSet::default());

        assert_eq!(order(&histories[0].done[&2], &histories[0].taken[&3], 10.0), Order::After);
    }

    #[test]
    fn sessions_whose_completed_set_was_never_read_are_dropped() {
        // Legacy traces: empty set all session long, although the character was level 20.
        let mut broken = make_episode();
        broken.levels = vec![TimedValue { t: 0.0, v: 20 }];
        broken.quest_log = vec![TimedValue { t: 0.0, v: vec![7] }];
        let (histories, report) = build_histories(&[&broken], &PARAMS, &FxHashSet::default());

        assert_eq!(report.without_completed, 1);
        assert!(histories.is_empty());
    }

    #[test]
    fn a_handful_of_removals_is_a_reset_but_a_dropped_out_completed_set_is_a_bad_read() {
        let mut reset = make_episode();
        reset.completed = Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: vec![1, 2, 3, 4, 5],
            changes: vec![CompletedChange {
                t: 10.0,
                add: None,
                remove: Some(vec![5]),
            }],
        });
        let mut bad_read = make_episode();
        bad_read.completed = Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: vec![1, 2, 3, 4, 6],
            changes: vec![
                CompletedChange {
                    t: 10.0,
                    add: None,
                    remove: Some(vec![1, 2, 3, 4, 6]),
                },
                CompletedChange {
                    t: 11.0,
                    add: Some(vec![1, 2, 3, 4, 6]),
                    remove: None,
                },
            ],
        });
        let unstable = unstable_quests(&[&reset, &bad_read], &catalog_of([]));

        assert_eq!(unstable.into_iter().collect::<Vec<_>>(), [5]);
    }
}
