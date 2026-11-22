//! objectKeys.name - Questie field 1. Port of `extract/observers/object/name.ts`; same shape
//! as npc/name.rs.
//!
//! UnitName(token) at each object encounter. `observe` keeps enUS names only;
//! `observe_localized` keeps every locale (for the lookup export).
//!
//! Looted objects are rarely targeted, so their names come from the object tooltip instead
//! (GameObjectName, Modules/Trackers/GameObject.lua): an object has to be hovered to be looted,
//! so the last tooltip before a loot names the looted object. Forever's tooltips have no GUID
//! (GameObjectGUID is ""), so the name cannot be checked against the looted object. The tracker
//! only records object tooltips, with player names sanitized out.

use crate::observation::{Confidence, Localized, Observation};
use crate::observers::object::encounters::object_encounters;
use crate::observers::object::loots::object_loots;
use crate::probes::{locale_at, unit_name_at};
use trace_data::stream::{get_stream, value_at};
use trace_data::{Session, Value};

pub fn observe_localized(session: &Session) -> Vec<Observation<Localized<String>>> {
    let mut observations = Vec::new();
    let mut observe = |object_id: i64, t: f64, name: Option<String>| {
        let (Some(locale), Some(name)) = (locale_at(session, t), name.filter(|name| !name.is_empty())) else {
            return;
        };
        let value = Localized {
            locale: locale.to_owned(),
            value: name,
        };
        observations.push(Observation::new(object_id, value, Confidence::High, t));
    };

    for encounter in object_encounters(session) {
        observe(encounter.object_id, encounter.t, unit_name_at(session, encounter.token, encounter.t));
    }
    if let Some(tooltips) = get_stream(session, "GameObjectName", &[]) {
        for loot in object_loots(session) {
            let name = value_at(tooltips, loot.t).and_then(Value::as_str);
            observe(loot.object_id, loot.t, name.map(str::to_owned));
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
    use crate::test_support::{event, session_json, stream, with_session};
    use serde_json::json;

    fn observed(functions: serde_json::Value) -> Vec<(i64, String, Confidence, f64)> {
        observed_with_events(functions, vec![])
    }

    fn observed_with_events(functions: serde_json::Value, events: Vec<serde_json::Value>) -> Vec<(i64, String, Confidence, f64)> {
        with_session(&session_json(functions, events), |s| {
            observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect()
        })
    }

    /// One loot of object 673206 at t 5.
    fn looted(tooltips: serde_json::Value) -> Vec<(i64, String, Confidence, f64)> {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "GameObjectName": tooltips,
            "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": { "1": "GameObject-0-5208-0-7-673206-0000399", "2": 1, "n": 2 } }] },
        });
        observed_with_events(functions, vec![event(5.0, "LOOT_READY", json!({ "n": 0 }))])
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

    #[test]
    fn names_a_looted_object_by_the_last_tooltip_before_the_loot() {
        let tooltips = stream([(2.0, json!("Mailbox")), (4.0, json!("Stolen Explosives")), (9.0, json!("Mailbox"))]);
        assert_eq!(looted(tooltips), [(673206, "Stolen Explosives".to_owned(), Confidence::High, 5.0)]);
    }

    #[test]
    fn does_not_name_a_looted_object_when_no_tooltip_came_before_the_loot() {
        assert_eq!(looted(stream([(9.0, json!("Stolen Explosives"))])), []);
    }
}
