//! The number of objectives C_QuestLog.GetQuestObjectives reports for a quest. Port of
//! `extract/observers/quest/objectiveCount.ts`.
//!
//! Not a Questie field: finalize uses it to drop questKeys.objectives when not every objective
//! could be matched to an id. Counting needs no text, so every locale is observed. "event"
//! objectives are exported as questKeys.triggerEnd and "log" entries carry no entity, so neither
//! counts. Every other type does, including ones the matcher cannot resolve (e.g. reputation):
//! exporting the remaining ids would leave the quest incomplete.

use super::objectives::emulated_list;
use crate::aggregate::Num;
use crate::js;
use crate::observation::{Confidence, Observation};
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{format_number, KeyRef, Session, Value};

const OBJECTIVES: &str = "C_QuestLog.GetQuestObjectives";
const NON_OBJECTIVE_TYPES: [&str; 2] = ["event", "log"];

/// JS `String(value)`, with None as `undefined`.
///
/// Shared by the quest observers that compare or match stringified values. It belongs in
/// `crate::js`; it lives here because that module is shared.
pub(super) fn js_string(value: Option<&Value>) -> String {
    let Some(value) = value else { return "undefined".to_owned() };
    match value.resolved() {
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => format_number(*n),
        Value::Str(s) => s.to_string(),
        // Array.prototype.join: decoded arrays hold no nil, so every item is stringified.
        Value::Array(items) => items.iter().map(|item| js_string(Some(item))).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_owned(),
        Value::Lazy(_) => unreachable!("resolved() never returns a lazy value"),
    }
}

pub fn observe(session: &Session) -> Vec<Observation<Num>> {
    let mut observations = Vec::new();
    let Some(root) = session.function(OBJECTIVES).filter(|root| !root.is_array()) else { return observations };

    for quest_id_key in param_keys(root) {
        let quest_id = js::number(&quest_id_key);
        if !quest_id.is_finite() {
            continue;
        }
        for entry in get_stream(session, OBJECTIVES, &[KeyRef::Str(&quest_id_key)]).unwrap_or(&[]) {
            let Some(objectives) = emulated_list(entry_v(entry)).filter(|list| !list.is_empty()) else { continue };
            let count = objectives
                .iter()
                .flatten()
                // `typeof objective === "object"`: arrays count too.
                .filter(|objective| objective.is_table() && !NON_OBJECTIVE_TYPES.contains(&js_string(objective.get("type")).as_str()))
                .count();
            observations.push(Observation::new(quest_id as i64, Num(count as f64), Confidence::High, entry_t(entry)));
        }
    }
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

    fn counts(functions: Json) -> Vec<f64> {
        observed(functions).into_iter().map(|(_, count, _, _)| count).collect()
    }

    #[test]
    fn observes_the_number_of_objectives_the_api_reports() {
        let functions = json!({
            "C_QuestLog.GetQuestObjectives": { "92109": stream([(3.0, json!([
                { "type": "item", "text": "0/5 Empty Vial" },
                { "type": "item", "text": "0/5 Peacebloom" },
                { "type": "item", "text": "0/5 Silverleaf" },
            ]))]) },
        });
        assert_eq!(observed(functions), [(92109, 3.0, Confidence::High, 3.0)]);
    }

    #[test]
    fn counts_objectives_of_non_en_us_sessions() {
        let functions = json!({
            "GetLocale": { "player": stream([(0.0, json!("deDE"))]) },
            "C_QuestLog.GetQuestObjectives": { "33": stream([(3.0, json!([{ "type": "item", "text": "Wolfsfleisch: 0/8" }]))]) },
        });
        assert_eq!(counts(functions), [1.0]);
    }

    #[test]
    fn does_not_count_event_or_log_objectives() {
        let functions = json!({
            "C_QuestLog.GetQuestObjectives": { "33": stream([(3.0, json!([
                { "type": "item", "text": "Tough Wolf Meat: 0/8" },
                { "type": "event", "text": "Light the campfire" },
                { "type": "log", "text": "Some flavor text" },
                { "type": "reputation", "text": "Friendly with the Argent Dawn" },
            ]))]) },
        });
        assert_eq!(counts(functions), [2.0]);
    }

    #[test]
    fn ignores_empty_objective_lists() {
        let functions = json!({ "C_QuestLog.GetQuestObjectives": { "33": stream([(3.0, json!([]))]) } });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn stringifies_types_like_js_string() {
        let types = json!({ "t": [true, 1.5, "event", ["event"], [1, ["a", 2]], { "k": 1 }] });
        let strings = crate::test_support::with_session(&session_json(types, vec![]), |s| {
            let items = s.function("t").and_then(Value::as_array).expect("an array");
            items.iter().map(|item| js_string(Some(item))).collect::<Vec<_>>()
        });
        assert_eq!(strings, ["true", "1.5", "event", "event", "1,a,2", "[object Object]"]);
        assert_eq!(js_string(None), "undefined");
    }
}
