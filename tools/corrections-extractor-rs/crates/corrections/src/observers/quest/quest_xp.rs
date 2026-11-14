//! `QuestXP.db[questId] = {questLevel, xp}`: Questie support data, not a questKeys field. Port
//! of `extract/observers/quest/questXp.ts`.
//!
//! Source: GetRewardXP, sampled in the quest dialog together with GetQuestID. The client reports
//! the XP scaled to the player's level, the database stores the base value. The two are equal
//! while the quest is at most XP_FULL_RANGE levels below the player, so only those samples are
//! merged; each keeps the player's level, compared with the quest level (a finalized fact) at
//! merge time. The client also rounds the XP it shows, so a sample is matched with the Forever
//! QuestXP table and the table's raw value is exported.

use crate::observation::{Confidence, Observation};
use crate::resources::{quest_xp_difficulty, quest_xp_for};
use indexmap::IndexMap;
use trace_data::stream::{entry_t, entry_tp, entry_v, get_stream, last_at_or_before, Stream};
use trace_data::{KeyRef, Session, Value};

/// One GetRewardXP sample. JS shape `{xp, playerLevel}`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuestXpValue {
    /// The XP the client showed.
    pub xp: f64,
    pub player_level: f64,
}

/// A quest more than this many levels below the player is scaled down.
const XP_FULL_RANGE: f64 = 5.0;

/// `v` of the latest entry recorded at or before `tp`, which orders samples taken at one `t`.
fn value_at_tp<'v, 'a>(stream: Stream<'v, 'a>, tp: f64) -> Option<&'v Value<'a>> {
    entry_v(&stream[last_at_or_before(stream.len(), |i| entry_tp(&stream[i]) <= tp)?])
}

pub fn observe(session: &Session) -> Vec<Observation<QuestXpValue>> {
    let quest_ids = get_stream(session, "GetQuestID", &[]).unwrap_or(&[]);
    let levels = get_stream(session, "UnitLevel", &[KeyRef::Str("player")]).unwrap_or(&[]);

    let mut observations = Vec::new();
    for entry in get_stream(session, "GetRewardXP", &[]).unwrap_or(&[]) {
        let Some(xp) = entry_v(entry).and_then(Value::as_num).filter(|&xp| xp > 0.0) else {
            continue;
        };
        let tp = entry_tp(entry);
        // TS skips `questId <= 0`, which lets NaN through.
        let Some(quest_id) = value_at_tp(quest_ids, tp).and_then(Value::as_num).filter(|&id| id > 0.0 || id.is_nan()) else {
            continue;
        };
        let Some(player_level) = value_at_tp(levels, tp).and_then(Value::as_num) else {
            continue;
        };
        observations.push(Observation::new(
            quest_id as i64,
            QuestXpValue { xp, player_level },
            Confidence::High,
            entry_t(entry),
        ));
    }
    observations
}

/// The raw XP of the difficulty most samples agree on. Only samples that are not scaled down and
/// match the rounded XP of one difficulty of `quest_level` in the Forever QuestXP table count;
/// buffed or mis-attributed samples match nothing. None without a quest level in the table, or
/// when the most observed difficulties tie.
pub fn merge(observations: &[Observation<QuestXpValue>], quest_level: Option<f64>) -> Option<f64> {
    let quest_level = quest_level?;

    let mut counts: IndexMap<usize, usize> = IndexMap::new();
    for observation in observations {
        if quest_level - observation.value.player_level < -XP_FULL_RANGE {
            continue;
        }
        if let Some(difficulty) = quest_xp_difficulty(quest_level, observation.value.xp) {
            *counts.entry(difficulty).or_default() += 1;
        }
    }

    let mut best = None;
    let mut best_count = 0;
    let mut tied = false;
    for (&difficulty, &count) in &counts {
        if count > best_count {
            best = Some(difficulty);
            best_count = count;
            tied = false;
        } else if count == best_count {
            tied = true;
        }
    }
    if tied {
        return None;
    }
    quest_xp_for(quest_level, best?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::{json, Value as Json};

    /// test/fixtures.ts `apiSample(time, preciseTime, value)`.
    fn api_sample(t: f64, tp: f64, value: f64) -> Json {
        json!({ "t": t, "tp": tp, "v": value })
    }

    fn observed(functions: Json) -> Vec<Observation<QuestXpValue>> {
        with_session(&session_json(functions, vec![]), observe)
    }

    fn observation(xp: f64, player_level: f64) -> Observation<QuestXpValue> {
        Observation::new(1, QuestXpValue { xp, player_level }, Confidence::High, 0.0)
    }

    #[test]
    fn pairs_the_reward_xp_with_the_quest_id_and_player_level_sampled_at_the_same_time() {
        let observations = observed(json!({
            "GetQuestID": [api_sample(0.0, 0.0, 0.0), api_sample(10.0, 10.5, 312.0), api_sample(10.0, 10.6, 0.0)],
            "GetRewardXP": [api_sample(0.0, 0.0, 0.0), api_sample(10.0, 10.5, 900.0), api_sample(10.0, 10.6, 0.0)],
            "UnitLevel": { "player": [api_sample(0.0, 0.0, 9.0)] },
        }));
        let observed: Vec<_> = observations.into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect();
        assert_eq!(observed, [(312, QuestXpValue { xp: 900.0, player_level: 9.0 }, Confidence::High, 10.0)]);
    }

    #[test]
    fn skips_rewards_without_a_quest_id_or_player_level() {
        let no_quest = observed(json!({
            "GetQuestID": [api_sample(0.0, 0.0, 0.0)],
            "GetRewardXP": [api_sample(5.0, 5.0, 900.0)],
            "UnitLevel": { "player": [api_sample(0.0, 0.0, 9.0)] },
        }));
        let no_level = observed(json!({
            "GetQuestID": [api_sample(5.0, 5.0, 312.0)],
            "GetRewardXP": [api_sample(5.0, 5.0, 900.0)],
        }));
        assert_eq!(no_quest, []);
        assert_eq!(no_level, []);
    }

    #[test]
    fn merge_exports_the_raw_table_xp_for_a_sample_the_client_rounded() {
        assert_eq!(merge(&[observation(900.0, 12.0)], Some(12.0)), Some(910.0));
        assert_eq!(merge(&[observation(525.0, 8.0)], Some(8.0)), Some(530.0));
    }

    #[test]
    fn merge_ignores_samples_that_match_no_difficulty_of_the_quest_level() {
        assert_eq!(merge(&[observation(1000.0, 12.0), observation(900.0, 12.0)], Some(12.0)), Some(910.0));
        assert_eq!(merge(&[observation(1000.0, 12.0)], Some(12.0)), None);
    }

    #[test]
    fn merge_ignores_samples_of_a_quest_more_than_five_levels_below_the_player() {
        assert_eq!(merge(&[observation(900.0, 17.0)], Some(12.0)), Some(910.0));
        assert_eq!(merge(&[observation(900.0, 18.0)], Some(12.0)), None);
    }

    #[test]
    fn merge_returns_the_difficulty_most_samples_agree_on() {
        assert_eq!(
            merge(&[observation(900.0, 12.0), observation(230.0, 12.0), observation(900.0, 12.0)], Some(12.0)),
            Some(910.0)
        );
    }

    #[test]
    fn merge_returns_nothing_when_the_most_observed_difficulties_tie() {
        assert_eq!(merge(&[observation(900.0, 12.0), observation(230.0, 12.0)], Some(12.0)), None);
    }

    #[test]
    fn merge_returns_nothing_without_a_quest_level_or_a_table_row_for_it() {
        assert_eq!(merge(&[observation(900.0, 12.0)], None), None);
        assert_eq!(merge(&[observation(900.0, 12.0)], Some(-1.0)), None);
    }
}
