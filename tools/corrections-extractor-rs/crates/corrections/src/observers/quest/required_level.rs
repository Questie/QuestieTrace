//! questKeys.requiredLevel - Questie field 4. Port of `extract/observers/quest/requiredLevel.ts`.
//!
//! The player's level at each QUEST_ACCEPTED, read from the UnitLevel("player") stream. The
//! lowest level across sessions wins.
//!
//! That stream only records changes, and UnitLevel("player") can still report the old level
//! inside PLAYER_LEVEL_UP, so it shows a new level up to about a second late. A quest accepted
//! in that gap would lower requiredLevel for good; the level PLAYER_LEVEL_UP announced is used
//! when it is higher. Levels never drop within a session.
//!
//! Traces before schema 18 also hold a QuestAcceptLevel stream. It is ignored: on Forever it
//! read QUEST_ACCEPTED's quest ID as a quest log index, recording levels against whichever
//! quest sat at that position.

use crate::observation::{Confidence, Observation};
use indexmap::IndexMap;
use trace_data::stream::{get_stream, value_at};
use trace_data::{KeyRef, Session, Value};

/// Era, TBC, Wrath and Mists send (questLogIndex, questID), Forever sends (questID) alone.
fn unpack_quest_id(args: Option<&Value>) -> Option<f64> {
    let args = args?.as_object()?;
    let count = args.get(KeyRef::Str("n"))?.as_num()?;
    // The quest id is the last of one or two arguments.
    if count != 1.0 && count != 2.0 {
        return None;
    }
    args.get(KeyRef::Num(count))?.as_num().filter(|&id| id != 0.0)
}

fn player_level_at(session: &Session, t: f64) -> Option<f64> {
    let stream = get_stream(session, "UnitLevel", &[KeyRef::Str("player")]).filter(|s| !s.is_empty())?;
    value_at(stream, t)?.as_num().filter(|level| level.is_finite())
}

pub fn observe(session: &Session) -> Vec<Observation<f64>> {
    // Insertion order: a quest keeps the position of its first acceptance.
    let mut accepted: IndexMap<i64, (f64, f64)> = IndexMap::new();
    // Announced by the newest PLAYER_LEVEL_UP so far.
    let mut level_up_level: Option<f64> = None;

    for event in session.events() {
        if event.is("PLAYER_LEVEL_UP") {
            if let Some(level) = event.arg(1).and_then(Value::as_num).filter(|l| l.is_finite()) {
                level_up_level = Some(level);
            }
            continue;
        }
        if !event.is("QUEST_ACCEPTED") {
            continue;
        }
        let Some(quest_id) = unpack_quest_id(event.args()).filter(|&id| id > 0.0) else {
            continue;
        };
        let known_levels = [player_level_at(session, event.t()), level_up_level];
        let Some(level) = known_levels.into_iter().flatten().reduce(f64::max) else {
            continue;
        };
        let entry = accepted.entry(quest_id as i64).or_insert((level, event.t()));
        if level < entry.0 {
            *entry = (level, event.t());
        }
    }

    accepted
        .into_iter()
        // Correlated from UnitLevel at acceptance event time.
        .map(|(quest_id, (level, t))| Observation::new(quest_id, level, Confidence::Medium, t))
        .collect()
}

/// The lowest acceptance level across sessions.
pub fn merge(observations: &[Observation<f64>]) -> f64 {
    observations.iter().map(|o| o.value).fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{event, packed, session_json, stream};

    fn accepted(t: f64, quest_id: f64) -> serde_json::Value {
        event(t, "QUEST_ACCEPTED", packed(&[quest_id.into()]))
    }

    fn levels(values: &[(f64, f64)]) -> serde_json::Value {
        serde_json::json!({ "UnitLevel": { "player": stream(values.iter().map(|&(t, v)| (t, v.into()))) } })
    }

    fn observed(session: &serde_json::Value) -> Vec<(i64, f64, f64)> {
        crate::test_support::with_session(session, |s| observe(s).into_iter().map(|o| (o.entity_id, o.value, o.t)).collect())
    }

    #[test]
    fn takes_the_lowest_player_level_at_quest_accepted_for_each_quest() {
        let session = session_json(
            levels(&[(0.0, 10.0), (5.0, 15.0), (10.0, 20.0)]),
            vec![accepted(3.0, 96659.0), accepted(7.0, 96659.0), accepted(12.0, 12345.0)],
        );
        assert_eq!(observed(&session), [(96659, 10.0, 3.0), (12345, 20.0, 12.0)]);
    }

    #[test]
    fn reads_the_quest_id_not_the_quest_log_index_from_an_era_quest_accepted() {
        let session = session_json(
            levels(&[(0.0, 12.0)]),
            vec![event(3.0, "QUEST_ACCEPTED", packed(&[3.0.into(), 96659.0.into()]))],
        );
        assert_eq!(observed(&session), [(96659, 12.0, 3.0)]);
    }

    #[test]
    fn ignores_the_quest_accept_level_stream_of_traces_before_schema_18() {
        // A real Forever pattern: accepting quest 14 recorded quest 237, the quest at log index 14.
        let mut functions = levels(&[(0.0, 21.0)]);
        functions["QuestAcceptLevel"] = serde_json::json!({ "237": stream([(3.0, 21.0.into())]) });
        let session = session_json(functions, vec![accepted(3.0, 14.0)]);
        assert_eq!(observed(&session), [(14, 21.0, 3.0)]);
    }

    #[test]
    fn uses_the_player_level_up_level_while_the_unit_level_stream_lags() {
        // Turning in a quest dings level 10; the follow-up is accepted before the stream catches up.
        let session = session_json(
            levels(&[(0.0, 9.0), (51.0, 10.0)]),
            vec![event(50.0, "PLAYER_LEVEL_UP", packed(&[10.0.into()])), accepted(50.2, 5001.0)],
        );
        assert_eq!(observed(&session), [(5001, 10.0, 50.2)]);
    }

    #[test]
    fn skips_accepts_without_any_known_level() {
        let session = session_json(serde_json::json!({}), vec![accepted(3.0, 96659.0)]);
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn merge_takes_the_minimum_across_sessions() {
        let obs = |level| Observation::new(96659, level, Confidence::Medium, 0.0);
        assert_eq!(merge(&[obs(30.0), obs(25.0), obs(28.0)]), 25.0);
    }
}
