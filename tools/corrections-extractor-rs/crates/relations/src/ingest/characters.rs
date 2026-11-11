//! Groups a contributor's sessions into characters without any player names or GUIDs.
//!
//! Candidates are a contributor's sessions with the same race, class and faction. Walking them in
//! start order, a session continues an existing character when it could be that character's next
//! session: it starts after the character's last session ended, at the same or a higher level, and
//! its completed-quest set at start contains (almost) everything the character had completed by
//! the end of its last session. Several characters can qualify early on (two same-race alts share
//! their starting-zone quests); the one with the longest history wins, because a short history
//! fits into a long one but not the other way round.
//!
//! Known failure modes:
//! - Two same race/class/faction alts that have completed (nearly) the same quests, e.g. two fresh
//!   level-1 alts, merge into one character until their histories diverge.
//! - A character whose completed history was reset or not loaded at capture start starts a new
//!   character key.
//! - Sessions without completed-quest data are only matched by level and time.
//! - Different contributor ids for one player (e.g. reinstalls) are never joined.

use super::short_hash;
use crate::model::{CompletedTimeline, Faction, PlayerContext, QuestId};
use indexmap::IndexMap;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterParams {
    /// Completed quests of the previous session that may be missing (daily/weekly resets).
    pub max_missing_quests: usize,
    pub max_missing_ratio: f64,
    /// Clock skew tolerated between one session's end and the next one's start.
    pub overlap_slack_seconds: f64,
}

pub const PARAMS: CharacterParams = CharacterParams {
    max_missing_quests: 5,
    max_missing_ratio: 0.05,
    overlap_slack_seconds: 120.0,
};

pub struct CharacterInput<'e> {
    pub episode_key: &'e str,
    pub contributor_id: &'e str,
    pub player: &'e PlayerContext,
    /// Epoch seconds at capture start (GetServerTime), when recorded.
    pub start: Option<f64>,
    pub duration: f64,
    /// Approximate epoch seconds used for ordering: `start`, else the session name stamp or the
    /// submission time. Always finite.
    pub order: f64,
    pub session_name: Option<&'e str>,
    pub first_level: Option<i64>,
    pub last_level: Option<i64>,
    pub completed: Option<&'e CompletedTimeline>,
}

impl CharacterInput<'_> {
    /// Completed quest ids at capture start, ascending.
    fn initial_completed(&self) -> Option<&[QuestId]> {
        self.completed.map(|completed| completed.initial.as_slice())
    }

    /// Completed quest ids at the end of the session, ascending: the initial set with every change
    /// applied. Worked out only for the session a character continues with, so a run never holds
    /// a second set per session.
    fn final_completed(&self) -> Option<Vec<QuestId>> {
        let completed = self.completed?;
        let mut set: BTreeSet<QuestId> = completed.initial.iter().copied().collect();
        for change in &completed.changes {
            set.extend(change.add.iter().flatten());
            for id in change.remove.iter().flatten() {
                set.remove(id);
            }
        }
        Some(set.into_iter().collect())
    }
}

struct Character {
    key: String,
    last_end: Option<f64>,
    last_level: Option<i64>,
    /// Completed quest ids at the end of its last session, ascending.
    completed: Option<Vec<QuestId>>,
}

impl Character {
    /// Its history's length; -1 without one, so any history beats none.
    fn history(&self) -> i64 {
        self.completed.as_ref().map_or(-1, |completed| completed.len() as i64)
    }
}

/// How many ids of sorted `a` are missing from sorted `b`.
fn count_missing(a: &[QuestId], b: &[QuestId]) -> usize {
    let mut missing = 0;
    let mut j = 0;
    for &id in a {
        while j < b.len() && b[j] < id {
            j += 1;
        }
        if j >= b.len() || b[j] != id {
            missing += 1;
        }
    }
    missing
}

fn fits(character: &Character, session: &CharacterInput) -> bool {
    if let (Some(last_end), Some(start)) = (character.last_end, session.start) {
        if start < last_end - PARAMS.overlap_slack_seconds {
            return false;
        }
    }
    if let (Some(last_level), Some(first_level)) = (character.last_level, session.first_level) {
        if first_level < last_level {
            return false;
        }
    }
    if let (Some(completed), Some(initial)) = (&character.completed, session.initial_completed()) {
        let allowed = (PARAMS.max_missing_quests as f64).max(PARAMS.max_missing_ratio * completed.len() as f64);
        if count_missing(completed, initial) as f64 > allowed {
            return false;
        }
    }
    true
}

fn faction_name(faction: Option<Faction>) -> Option<&'static str> {
    faction.map(|faction| match faction {
        Faction::Alliance => "Alliance",
        Faction::Horde => "Horde",
        Faction::Neutral => "Neutral",
    })
}

/// The character key of each session, by position.
pub fn assign_characters(sessions: &[CharacterInput]) -> Vec<String> {
    let mut groups: IndexMap<_, Vec<usize>> = IndexMap::new();
    for (index, session) in sessions.iter().enumerate() {
        let PlayerContext { race_id, class_id, faction } = session.player;
        groups
            .entry((session.contributor_id, *race_id, *class_id, faction_name(*faction)))
            .or_default()
            .push(index);
    }

    let mut keys = vec![String::new(); sessions.len()];
    for mut group in groups.into_values() {
        group.sort_by(|&a, &b| {
            let (a, b) = (&sessions[a], &sessions[b]);
            a.order.total_cmp(&b.order).then_with(|| a.episode_key.cmp(b.episode_key))
        });
        let mut characters: Vec<Character> = Vec::new();
        for index in group {
            let session = &sessions[index];
            let mut best: Option<usize> = None;
            for (candidate, character) in characters.iter().enumerate() {
                if fits(character, session) && best.is_none_or(|best| character.history() > characters[best].history()) {
                    best = Some(candidate);
                }
            }
            let best = best.unwrap_or_else(|| {
                characters.push(Character {
                    key: short_hash(&format!("character|{}|{}", session.contributor_id, session.episode_key)),
                    last_end: None,
                    last_level: None,
                    completed: None,
                });
                characters.len() - 1
            });
            let character = &mut characters[best];
            if let Some(start) = session.start {
                character.last_end = Some(start + session.duration);
            }
            if session.last_level.is_some() {
                character.last_level = session.last_level;
            }
            if let Some(completed) = session.final_completed() {
                character.completed = Some(completed);
            }
            keys[index].clone_from(&character.key);
        }
    }
    keys
}

/// Each session's 0-based position among its character's sessions, oldest first
/// (QuestEpisode.sessionOrder). Ordered by progress (completed history size at capture start),
/// then capture start time, which unlike the session name exists for unsaved sessions too, then
/// name and key for determinism. Names all have one format, so comparing them as text orders
/// them like the TS localeCompare.
pub fn order_sessions(sessions: &[CharacterInput], characters: &[String]) -> Vec<u32> {
    let mut by_character: IndexMap<&str, Vec<usize>> = IndexMap::new();
    for (index, character) in characters.iter().enumerate() {
        by_character.entry(character).or_default().push(index);
    }
    let progress = |session: &CharacterInput| session.initial_completed().map_or(0, <[QuestId]>::len);
    let mut order = vec![0; sessions.len()];
    for mut list in by_character.into_values() {
        list.sort_by(|&a, &b| {
            let (a, b) = (&sessions[a], &sessions[b]);
            progress(a)
                .cmp(&progress(b))
                .then_with(|| a.order.total_cmp(&b.order))
                .then_with(|| a.session_name.unwrap_or_default().cmp(b.session_name.unwrap_or_default()))
                .then_with(|| a.episode_key.cmp(b.episode_key))
        });
        for (position, index) in list.into_iter().enumerate() {
            order[index] = position as u32;
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CompletedChange, CompletedSource};

    const ORC_WARRIOR: PlayerContext = PlayerContext {
        race_id: Some(2),
        class_id: Some(1),
        faction: Some(Faction::Horde),
    };

    fn range(from: QuestId, to: QuestId) -> Vec<QuestId> {
        (from..=to).collect()
    }

    /// Completed `initial` at capture start and `last` at the end (never fewer).
    fn history(initial: Vec<QuestId>, last: Vec<QuestId>) -> CompletedTimeline {
        let added = last.iter().copied().filter(|id| !initial.contains(id)).collect();
        CompletedTimeline {
            source: CompletedSource::Modern,
            initial,
            changes: vec![CompletedChange {
                t: 1.0,
                add: Some(added),
                remove: None,
            }],
        }
    }

    fn session<'e>(key: &'e str, start: f64, levels: (i64, i64), completed: &'e CompletedTimeline) -> CharacterInput<'e> {
        CharacterInput {
            episode_key: key,
            contributor_id: "contributor",
            player: &ORC_WARRIOR,
            start: Some(start),
            duration: 3600.0,
            order: start,
            session_name: None,
            first_level: Some(levels.0),
            last_level: Some(levels.1),
            completed: Some(completed),
        }
    }

    #[test]
    fn keeps_a_same_race_and_class_alt_apart_from_the_main_even_though_the_main_did_all_its_quests() {
        // Both start in the same zone, so the alt's quests 1..10 are a subset of the main's history.
        let histories = [
            history(range(1, 300), range(1, 310)),
            history(vec![], range(1, 10)),
            history(range(1, 310), range(1, 320)),
            history(range(1, 10), range(1, 20)),
        ];
        let sessions = [
            session("main-1", 0.0, (30, 31), &histories[0]),
            session("alt-1", 10_000.0, (1, 5), &histories[1]),
            session("main-2", 20_000.0, (31, 32), &histories[2]),
            session("alt-2", 30_000.0, (5, 8), &histories[3]),
        ];
        let keys = assign_characters(&sessions);
        assert_eq!(keys[2], keys[0], "main-2 continues main-1");
        assert_eq!(keys[3], keys[1], "alt-2 continues alt-1");
        assert_ne!(keys[1], keys[0]);
    }

    #[test]
    fn splits_sessions_that_overlap_in_time_or_would_lose_completed_quests() {
        let histories = [
            history(range(1, 50), range(1, 60)),
            history(range(1, 60), range(1, 60)),
            history(range(1, 20), range(1, 20)),
        ];
        let sessions = [
            session("a", 0.0, (10, 10), &histories[0]),
            session("overlapping", 1800.0, (10, 10), &histories[1]),
            session("lost-history", 10_000.0, (12, 12), &histories[2]),
        ];
        let keys = assign_characters(&sessions);
        assert!(keys[0] != keys[1] && keys[1] != keys[2] && keys[0] != keys[2], "{keys:?}");
    }

    #[test]
    fn a_session_ends_with_its_initial_set_and_every_change_applied() {
        // Quest 2 is a daily the reset removed; 5 was completed during the session.
        let completed = CompletedTimeline {
            source: CompletedSource::Modern,
            initial: vec![1, 2, 3],
            changes: vec![
                CompletedChange {
                    t: 1.0,
                    add: Some(vec![5]),
                    remove: None,
                },
                CompletedChange {
                    t: 2.0,
                    add: None,
                    remove: Some(vec![2]),
                },
            ],
        };
        assert_eq!(session("s", 0.0, (1, 1), &completed).final_completed(), Some(vec![1, 3, 5]));
    }

    #[test]
    fn orders_a_characters_sessions_by_progress_then_start_time_whether_or_not_they_have_a_name() {
        let histories = [
            history(range(1, 40), range(1, 45)),
            history(range(1, 45), range(1, 50)),
            history(range(1, 40), range(1, 40)),
        ];
        let named = CharacterInput {
            session_name: Some("2026-10-01_12-00-00"),
            ..session("named", 2000.0, (10, 10), &histories[0])
        };
        // In-progress exports carry no name; this one started later with more history.
        let unnamed_later = session("unnamed-later", 9000.0, (10, 11), &histories[1]);
        // Same progress as `named` but captured earlier: the start time breaks the tie.
        let unnamed_tie = session("unnamed-tie", 1000.0, (10, 10), &histories[2]);
        let sessions = [unnamed_later, named, unnamed_tie];
        let order = order_sessions(&sessions, &vec!["character".to_string(); 3]);
        assert_eq!(order, [2, 1, 0]);
    }
}
