//! questKeys.questFlags and specialFlags of daily quests - Questie fields 23 and 24. Not in the
//! TypeScript extractor.
//!
//! Source: C_QuestLog.GetInfo(questID).frequency, sampled while the quest is in the log
//! (Modules/Trackers/QuestLog.lua). It is Enum.QuestFrequency: 0 default, 1 daily, 2 weekly, 3
//! reset by scheduler.
//!
//! Only daily is exported. Checked against the hand-written flags: all 144 hand-written dailies
//! seen in 56,138 sessions were daily here, none of the 4,181 other quests was, and no quest had
//! two values. The three quests with 3 ("In Demand - ...") have no flag in Questie to compare with.

use crate::aggregate::Num;
use crate::observation::{Confidence, Observation};
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

/// Enum.QuestFrequency.Daily
pub const DAILY: f64 = 1.0;

/// Questie's questFlags.DAILY and specialFlags.REPEATABLE.
pub const QUEST_FLAG_DAILY: i64 = 4096;
pub const SPECIAL_FLAG_REPEATABLE: i64 = 1;

pub fn observe(session: &Session) -> Vec<Observation<Num>> {
    let Some(root) = session.function("C_QuestLog.GetInfo") else {
        return Vec::new();
    };
    let mut observations = Vec::new();
    for quest_id_key in param_keys(root) {
        let Ok(quest_id) = quest_id_key.parse::<i64>() else { continue };
        for entry in get_stream(session, "C_QuestLog.GetInfo", &[KeyRef::Str(&quest_id_key)]).unwrap_or_default() {
            let Some(frequency) = entry_v(entry).and_then(|info| info.get("frequency")).and_then(Value::as_num) else {
                continue;
            };
            observations.push(Observation::new(quest_id, Num(frequency), Confidence::High, entry_t(entry)));
        }
    }
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::{json, Value as Json};

    fn observed(functions: Json) -> Vec<(i64, f64, f64)> {
        with_session(&session_json(functions, vec![]), |s| {
            observe(s).into_iter().map(|o| (o.entity_id, o.value.0, o.t)).collect()
        })
    }

    #[test]
    fn observes_the_frequency_of_every_quest_in_the_log() {
        let functions = json!({ "C_QuestLog.GetInfo": {
            "94004": [{ "t": 2, "tp": 2, "v": { "questID": 94004, "frequency": 1 } }],
            "96659": [{ "t": 3, "tp": 3, "v": { "questID": 96659, "frequency": 0 } }],
        }});
        assert_eq!(observed(functions), [(94004, 1.0, 2.0), (96659, 0.0, 3.0)]);
    }

    #[test]
    fn skips_reads_without_a_frequency() {
        let functions = json!({ "C_QuestLog.GetInfo": {
            "94004": [{ "t": 2, "tp": 2, "v": { "questID": 94004 } }, { "t": 3, "tp": 3 }],
        }});
        assert_eq!(observed(functions), []);
        assert_eq!(observed(json!({})), []);
    }
}
