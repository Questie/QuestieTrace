//! objectKeys.name - Questie field 1. Port of `extract/observers/object/name.ts`; same shape
//! as npc/name.rs.
//!
//! UnitName(token) at each object encounter. `observe` keeps enUS names only;
//! `observe_localized` keeps every locale (for the lookup export).

use crate::observation::{Confidence, Localized, Observation};
use crate::observers::object::encounters::object_encounters;
use crate::probes::{locale_at, unit_name_at};
use trace_data::Session;

pub fn observe_localized(session: &Session) -> Vec<Observation<Localized<String>>> {
    let mut observations = Vec::new();
    for encounter in object_encounters(session) {
        let Some(locale) = locale_at(session, encounter.t) else { continue };
        if let Some(name) = unit_name_at(session, encounter.token, encounter.t) {
            let value = Localized {
                locale: locale.to_owned(),
                value: name,
            };
            observations.push(Observation::new(encounter.object_id, value, Confidence::High, encounter.t));
        }
    }
    observations
}

pub fn observe(session: &Session) -> Vec<Observation<String>> {
    observe_localized(session)
        .into_iter()
        .filter(|obs| obs.value.locale == "enUS")
        .map(|obs| obs.map(|name| name.value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::json;

    fn observed(functions: serde_json::Value) -> Vec<(i64, String, Confidence, f64)> {
        with_session(&session_json(functions, vec![]), |s| {
            observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect()
        })
    }

    #[test]
    fn observes_the_name_at_the_time_and_token_of_the_guid_encounter() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "UnitGUID": { "target": [{ "t": 3, "tp": 3, "v": "GameObject-0-5208-0-7-2843-0000399" }] },
            "UnitName": { "target": [{ "t": 3, "tp": 3, "v": { "1": "Suspicious Chest", "n": 2 } }] },
        });
        assert_eq!(observed(functions), [(2843, "Suspicious Chest".to_owned(), Confidence::High, 3.0)]);
    }

    #[test]
    fn skips_encounters_without_an_observed_name() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "UnitGUID": { "target": [{ "t": 3, "tp": 3, "v": "GameObject-0-5208-0-7-2843-0000399" }] },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn skips_encounters_under_a_non_en_us_locale() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "frFR" }] },
            "UnitGUID": { "target": [{ "t": 3, "tp": 3, "v": "GameObject-0-5208-0-7-2843-0000399" }] },
            "UnitName": { "target": [{ "t": 3, "tp": 3, "v": { "1": "Coffre suspect", "n": 2 } }] },
        });
        assert_eq!(observed(functions), []);
    }
}
