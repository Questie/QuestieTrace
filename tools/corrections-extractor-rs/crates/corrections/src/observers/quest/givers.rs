//! Which quest givers are exported, shared by the startedBy and finishedBy merges. Port of
//! `extract/merge/quest/relations.ts`.
//!
//! A giver seen in few sessions is more likely a misattributed one (a stale questnpc, say) than a
//! real one, and a wrong giver does more harm than a missing one. The same holds for a giver seen
//! in a tiny share of the sessions of the quest's strongest giver, such as an object that only
//! replaces the real starter after the previous quest was turned in.

use crate::observation::{Observation, SessionId};
use std::collections::{BTreeMap, HashSet};

/// A giver is exported only once this many sessions saw it.
pub const MIN_GIVER_SESSIONS: usize = 5;
/// ... and at least this share of the sessions of the quest's strongest giver, of any kind.
pub const MIN_GIVER_SHARE: f64 = 0.005;

/// Per giver kind (`KINDS` of them), the ids seen in at least [`MIN_GIVER_SESSIONS`] sessions and
/// in at least [`MIN_GIVER_SHARE`] of the strongest giver's sessions, ascending. `giver` gives an
/// observation's kind (an index below `KINDS`) and id; id 0 is no giver.
pub fn supported_ids<T, const KINDS: usize>(observations: &[Observation<T>], giver: impl Fn(&T) -> (usize, i64)) -> [Vec<i64>; KINDS] {
    let mut sessions_by_id: [BTreeMap<i64, HashSet<SessionId>>; KINDS] = std::array::from_fn(|_| BTreeMap::new());
    for obs in observations {
        let (kind, id) = giver(&obs.value);
        if id != 0 {
            sessions_by_id[kind].entry(id).or_default().insert(obs.session);
        }
    }
    let strongest = sessions_by_id.iter().flat_map(|by_id| by_id.values().map(HashSet::len)).max().unwrap_or(0);
    sessions_by_id.map(|by_id| {
        by_id
            .into_iter()
            .filter(|(_, sessions)| sessions.len() >= MIN_GIVER_SESSIONS && sessions.len() as f64 >= strongest as f64 * MIN_GIVER_SHARE)
            .map(|(id, _)| id)
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use crate::observation::{Confidence, Observation, SessionId};
    use crate::observers::quest::finished_by::{self, FinishedByFinisher, FinishedByValue};
    use crate::observers::quest::started_by::{self, StartedByStarter, StartedByValue};

    /// `value` seen in `sessions` sessions, `per_session` times in each (relations.test.ts `seen`).
    fn seen<T: Clone>(value: T, sessions: u32, per_session: u32) -> Vec<Observation<T>> {
        (0..sessions * per_session)
            .map(|i| Observation {
                session: SessionId(i / per_session),
                ..Observation::new(1, value.clone(), Confidence::Medium, f64::from(i))
            })
            .collect()
    }

    fn finishers(groups: &[(FinishedByFinisher, u32, u32)]) -> Option<FinishedByValue> {
        let observations: Vec<_> = groups.iter().flat_map(|&(value, sessions, per)| seen(value, sessions, per)).collect();
        finished_by::merge(&observations)
    }

    fn starters(groups: &[(StartedByStarter, u32, u32)]) -> Option<StartedByValue> {
        let observations: Vec<_> = groups.iter().flat_map(|&(value, sessions, per)| seen(value, sessions, per)).collect();
        started_by::merge(&observations)
    }

    fn finished(creatures: &[i64], objects: &[i64]) -> Option<FinishedByValue> {
        Some(FinishedByValue {
            creatures: creatures.to_vec(),
            objects: objects.to_vec(),
        })
    }

    fn started(creatures: &[i64], objects: &[i64], items: &[i64]) -> Option<StartedByValue> {
        Some(StartedByValue {
            creatures: creatures.to_vec(),
            objects: objects.to_vec(),
            items: items.to_vec(),
        })
    }

    use FinishedByFinisher as F;
    use StartedByStarter as S;

    // describe("mergeFinishedBy session support")

    #[test]
    fn keeps_finishers_seen_in_at_least_5_sessions() {
        assert_eq!(finishers(&[(F::Creature(1077), 5, 1), (F::Object(2843), 6, 1)]), finished(&[1077], &[2843]));
    }

    #[test]
    fn drops_finishers_seen_in_fewer_than_5_sessions_however_often() {
        assert_eq!(finishers(&[(F::Creature(1077), 5, 1), (F::Creature(1076), 4, 10)]), finished(&[1077], &[]));
    }

    #[test]
    fn gives_no_finisher_value_when_no_finisher_has_enough_sessions() {
        assert_eq!(finishers(&[(F::Creature(1076), 4, 1)]), None);
    }

    // describe("mergeStartedBy session support")

    #[test]
    fn keeps_starters_seen_in_at_least_5_sessions() {
        assert_eq!(
            starters(&[(S::Creature(197), 5, 1), (S::Object(2843), 5, 1), (S::Item(750), 5, 1)]),
            started(&[197], &[2843], &[750])
        );
    }

    #[test]
    fn drops_starters_seen_in_fewer_than_5_sessions_however_often() {
        assert_eq!(
            starters(&[(S::Creature(197), 5, 1), (S::Creature(200), 4, 10), (S::Item(750), 1, 1)]),
            started(&[197], &[], &[])
        );
    }

    #[test]
    fn gives_no_starter_value_when_no_starter_has_enough_sessions() {
        assert_eq!(starters(&[(S::Item(750), 4, 1)]), None);
    }

    // describe("quest giver share of the strongest giver")

    #[test]
    fn drops_a_starter_seen_in_under_half_a_percent_of_the_strongest_starters_sessions() {
        assert_eq!(
            starters(&[(S::Creature(265813), 1001, 1), (S::Creature(1252), 5, 1)]),
            started(&[265813], &[], &[])
        );
    }

    #[test]
    fn keeps_a_starter_seen_in_half_a_percent_of_the_strongest_starters_sessions() {
        assert_eq!(
            starters(&[(S::Creature(265813), 1000, 1), (S::Object(2843), 5, 1)]),
            started(&[265813], &[2843], &[])
        );
    }

    #[test]
    fn drops_a_finisher_seen_in_under_half_a_percent_of_the_strongest_finishers_sessions() {
        assert_eq!(finishers(&[(F::Creature(4453), 1001, 1), (F::Object(2843), 5, 1)]), finished(&[4453], &[]));
    }

    #[test]
    fn compares_a_starter_with_the_strongest_starter_of_any_kind() {
        assert_eq!(starters(&[(S::Item(750), 1001, 1), (S::Creature(1252), 5, 1)]), started(&[], &[], &[750]));
    }

    #[test]
    fn lists_the_supported_ids_of_each_kind_ascending() {
        assert_eq!(
            starters(&[(S::Creature(300), 5, 1), (S::Creature(197), 5, 1), (S::Creature(250), 6, 1)]),
            started(&[197, 250, 300], &[], &[])
        );
    }
}
