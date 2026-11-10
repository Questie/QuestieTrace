//! questKeys.questLevel - Questie field 5. Port of `extract/observers/quest/questLevel.ts`.
//!
//! Read from two streams, both keyed by questID:
//!   - GetQuestLogTitle(questID) tuple position 2 (the level)
//!   - C_QuestLog.GetInfo(questID).level (modern API, table return)
//!
//! These are quest-log streams: quests can be in the log without GetQuestID() ever being
//! sampled, so the streams are observed directly.

use super::objectives::emulated_list;
use crate::aggregate::Num;
use crate::js;
use crate::observation::{Confidence, Observation};
use std::collections::HashSet;
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

/// A finite number, or a table's finite `level`.
fn read_level(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    if let Some(level) = value.as_num() {
        return level.is_finite().then_some(level);
    }
    value.get("level")?.as_num().filter(|level| level.is_finite())
}

fn read_get_quest_log_title_level(value: Option<&Value>) -> Option<f64> {
    let tuple = emulated_list(value)?;
    // Position 2 (index 1) is the level.
    read_level(*tuple.get(1)?)
}

fn read_get_info_level(value: Option<&Value>) -> Option<f64> {
    read_level(value?.as_object()?.get(KeyRef::Str("level")))
}

/// Appends the levels of every quest in `function_name`'s streams, except `skip_keys`, sorted
/// by time. Returns the keys of the quests that had a level.
fn append_stream_levels(
    session: &Session,
    observations: &mut Vec<Observation<Num>>,
    function_name: &str,
    read_entry: fn(Option<&Value>) -> Option<f64>,
    skip_keys: &HashSet<String>,
) -> HashSet<String> {
    let mut observed_keys = HashSet::new();
    let Some(root) = session.function(function_name).filter(|root| !root.is_array()) else { return observed_keys };

    let mut pending = Vec::new();
    for quest_id_key in param_keys(root) {
        if skip_keys.contains(&quest_id_key) {
            continue;
        }
        let quest_id = js::number(&quest_id_key);
        if !quest_id.is_finite() {
            continue;
        }
        let mut has_level = false;
        for entry in get_stream(session, function_name, &[KeyRef::Str(&quest_id_key)]).unwrap_or(&[]) {
            let Some(level) = read_entry(entry_v(entry)) else { continue };
            has_level = true;
            pending.push(Observation::new(quest_id as i64, Num(level), Confidence::High, entry_t(entry)));
        }
        if has_level {
            observed_keys.insert(quest_id_key);
        }
    }

    // Stable, like `(a, b) => a.t - b.t`. A total order (-0 as 0, NaN last) so a missing time
    // cannot make the sort panic; JS leaves the order of NaN times to the engine.
    pending.sort_by(|a, b| (a.t + 0.0).total_cmp(&(b.t + 0.0)));
    observations.extend(pending);
    observed_keys
}

pub fn observe(session: &Session) -> Vec<Observation<Num>> {
    let mut observations = Vec::new();
    // Prefer the modern API when both recorded the same quest.
    let modern_keys = append_stream_levels(session, &mut observations, "C_QuestLog.GetInfo", read_get_info_level, &HashSet::new());
    append_stream_levels(session, &mut observations, "GetQuestLogTitle", read_get_quest_log_title_level, &modern_keys);
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, stream, with_session};
    use serde_json::{json, Value as Json};

    fn observed(functions: Json) -> Vec<(i64, f64, Confidence, f64)> {
        with_session(&session_json(functions, vec![]), |s| observe(s).into_iter().map(|o| (o.entity_id, o.value.0, o.confidence, o.t)).collect())
    }

    #[test]
    fn reads_the_level_from_get_quest_log_title_tuple_position_2() {
        let functions = json!({
            "GetQuestID": stream([(3.0, json!(96659))]),
            "GetQuestLogTitle": { "96659": stream([(3.0, json!({ "1": "A Threat Within", "2": 60, "3": 0, "4": false, "5": false, "6": 0, "7": 1, "8": 96659, "n": 8 }))]) },
        });
        assert_eq!(observed(functions), [(96659, 60.0, Confidence::High, 3.0)]);
    }

    #[test]
    fn reads_the_level_from_the_get_info_level_field() {
        let functions = json!({
            "GetQuestID": stream([(3.0, json!(96659))]),
            "C_QuestLog.GetInfo": { "96659": stream([(3.0, json!({ "questID": 96659, "level": 55, "isHeader": false, "title": "A Threat Within" }))]) },
        });
        assert_eq!(observed(functions), [(96659, 55.0, Confidence::High, 3.0)]);
    }

    #[test]
    fn reads_the_level_of_a_quest_without_a_get_quest_id_encounter() {
        let functions = json!({
            "C_QuestLog.GetInfo": { "98013": stream([(2.0, json!({ "questID": 98013, "level": 80, "title": "Swelling Forces" }))]) },
        });
        assert_eq!(observed(functions), [(98013, 80.0, Confidence::High, 2.0)]);
    }

    #[test]
    fn reads_the_legacy_level_of_a_quest_without_a_get_quest_id_encounter() {
        let functions = json!({
            "GetQuestLogTitle": { "98013": stream([(2.0, json!({ "1": "Swelling Forces", "2": 80, "8": 98013, "n": 8 }))]) },
        });
        let (entity_id, level, _, _) = observed(functions)[0];
        assert_eq!((entity_id, level), (98013, 80.0));
    }

    #[test]
    fn prefers_get_info_over_get_quest_log_title_when_both_exist() {
        let functions = json!({
            "GetQuestID": stream([(3.0, json!(96659))]),
            "GetQuestLogTitle": { "96659": stream([(3.0, json!({ "1": "A Threat Within", "2": 60, "8": 96659, "n": 8 }))]) },
            "C_QuestLog.GetInfo": { "96659": stream([(3.0, json!({ "questID": 96659, "level": 55, "isHeader": false, "title": "A Threat Within" }))]) },
        });
        assert_eq!(observed(functions), [(96659, 55.0, Confidence::High, 3.0)]);
    }

    #[test]
    fn observes_nothing_without_a_level_stream() {
        assert_eq!(observed(json!({ "GetQuestID": stream([(3.0, json!(96659))]) })), []);
    }

    #[test]
    fn observes_nothing_when_get_info_has_no_level_field() {
        let functions = json!({
            "GetQuestID": stream([(3.0, json!(96659))]),
            "C_QuestLog.GetInfo": { "96659": stream([(3.0, json!({ "questID": 96659, "isHeader": false, "title": "A Threat Within" }))]) },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn skips_entries_whose_level_is_not_a_finite_number() {
        let functions = json!({
            "GetQuestID": stream([(3.0, json!(96659))]),
            "GetQuestLogTitle": { "96659": stream([(3.0, json!({ "1": "A Threat Within", "2": null, "n": 2 }))]) },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn observes_several_quests_of_one_session_in_time_order() {
        // JS iterates the key "12345" before "96659"; the observations come out by time.
        let functions = json!({
            "GetQuestID": stream([(3.0, json!(96659)), (7.0, json!(12345))]),
            "GetQuestLogTitle": {
                "96659": stream([(3.0, json!({ "1": "A Threat Within", "2": 60, "8": 96659, "n": 8 }))]),
                "12345": stream([(7.0, json!({ "1": "Arya's Request", "2": 45, "8": 12345, "n": 8 }))]),
            },
        });
        assert_eq!(observed(functions), [(96659, 60.0, Confidence::High, 3.0), (12345, 45.0, Confidence::High, 7.0)]);
    }
}
