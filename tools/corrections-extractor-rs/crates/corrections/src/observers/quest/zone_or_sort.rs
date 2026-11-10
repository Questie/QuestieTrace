//! questKeys.zoneOrSort - Questie field 17. Port of `extract/observers/quest/zoneOrSort.ts`.
//!
//! QuestLogZone records the quest-log header shown above each active quest: an AreaTable name
//! for zone groups, a QuestSort name for special groups. The title is turned back into the
//! signed DB2 id Questie expects (areas positive, sorts negative).

use crate::aggregate::Num;
use crate::js;
use crate::observation::{Confidence, Observation};
use crate::resources::zone_or_sort_id_for_name;
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

pub fn observe(session: &Session) -> Vec<Observation<Num>> {
    let mut observations = Vec::new();
    let Some(root) = session.function("QuestLogZone").filter(|root| !root.is_array()) else { return observations };

    for quest_id_key in param_keys(root) {
        let quest_id = js::number(&quest_id_key);
        if !quest_id.is_finite() {
            continue;
        }
        for entry in get_stream(session, "QuestLogZone", &[KeyRef::Str(&quest_id_key)]).unwrap_or(&[]) {
            let Some(zone_or_sort) = entry_v(entry).and_then(Value::as_str).and_then(zone_or_sort_id_for_name) else { continue };
            // The quest-log association is observed, but the id is resolved through the bundled
            // DB2 name lookup rather than returned by WoW.
            observations.push(Observation::new(quest_id as i64, Num(zone_or_sort), Confidence::Medium, entry_t(entry)));
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

    #[test]
    fn maps_a_quest_log_zone_header_to_its_area_id() {
        let functions = json!({ "QuestLogZone": { "96659": stream([(2.0, json!("Westfall"))]) } });
        assert_eq!(observed(functions), [(96659, 40.0, Confidence::Medium, 2.0)]);
    }

    #[test]
    fn processes_every_quest_log_entry_and_skips_nonnumeric_quest_keys() {
        let functions = json!({
            "QuestLogZone": {
                "96659": stream([(2.0, json!("Westfall")), (7.0, json!("Epic"))]),
                "not-a-quest": stream([(4.0, json!("Westfall"))]),
            },
        });
        assert_eq!(observed(functions), [(96659, 40.0, Confidence::Medium, 2.0), (96659, -1.0, Confidence::Medium, 7.0)]);
    }

    #[test]
    fn maps_a_quest_log_special_header_to_a_negative_quest_sort_id() {
        let functions = json!({ "QuestLogZone": { "96659": stream([(2.0, json!("Epic"))]) } });
        assert_eq!(observed(functions)[0].1, -1.0);
    }

    #[test]
    fn skips_unknown_or_missing_header_titles() {
        let functions = json!({ "QuestLogZone": { "96659": stream([(2.0, json!("Not a DB2 header"))]) } });
        assert_eq!(observed(functions), []);
    }
}
