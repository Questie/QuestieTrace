//! itemKeys.name - Questie field 1. Port of `extract/observers/item/name.ts`.
//!
//! Unlike npc and object names, the item id and name come from one signal: the
//! `GetLootSlotLink(slot)` item hyperlink (see Modules/Trackers/Loot.lua), so no GUID or
//! time correlation is needed. `observe` keeps enUS names only; `observe_localized` keeps
//! every locale (for the lookup export).

use crate::item_link::parse_item_link;
use crate::observation::{Confidence, Localized, Observation};
use crate::probes::locale_at;
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

pub fn observe_localized(session: &Session) -> Vec<Observation<Localized<String>>> {
    let mut observations = Vec::new();
    let Some(link_stream) = session.function("GetLootSlotLink") else { return observations };
    for slot in param_keys(link_stream) {
        let Some(entries) = get_stream(session, "GetLootSlotLink", &[KeyRef::Str(&slot)]) else { continue };
        for entry in entries {
            let Some(link) = entry_v(entry).and_then(Value::as_str) else { continue };
            let t = entry_t(entry);
            let Some(locale) = locale_at(session, t) else { continue };
            let Some(parsed) = parse_item_link(Some(link)) else { continue };
            let value = Localized { locale: locale.to_owned(), value: parsed.name.to_owned() };
            observations.push(Observation::new(parsed.item_id as i64, value, Confidence::High, t));
        }
    }
    observations
}

pub fn observe(session: &Session) -> Vec<Observation<String>> {
    observe_localized(session).into_iter().filter(|obs| obs.value.locale == "enUS").map(|obs| obs.map(|name| name.value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::json;

    fn observed(functions: serde_json::Value) -> Vec<(i64, String, Confidence, f64)> {
        with_session(&session_json(functions, vec![]), |s| observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect())
    }

    #[test]
    fn observes_the_item_id_and_name_from_a_loot_slot_link() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "GetLootSlotLink": { "1": [{ "t": 3, "tp": 3, "v": "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }] },
        });
        assert_eq!(observed(functions), [(750, "Tough Wolf Meat".to_owned(), Confidence::High, 3.0)]);
    }

    #[test]
    fn observes_every_slot_across_loot_events() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "GetLootSlotLink": {
                "1": [{ "t": 0, "tp": 0, "v": "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }],
                "2": [{ "t": 0, "tp": 0, "v": "|Hitem:2604::::::::1::::::::::|h[Kobold Mining Bag]|h[|r" }],
            },
        });
        assert_eq!(
            observed(functions),
            [(750, "Tough Wolf Meat".to_owned(), Confidence::High, 0.0), (2604, "Kobold Mining Bag".to_owned(), Confidence::High, 0.0)]
        );
    }

    #[test]
    fn skips_slot_entries_that_are_not_an_item_link() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] },
            "GetLootSlotLink": { "1": [{ "t": 0, "tp": 0, "v": null }] },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn returns_nothing_without_a_loot_slot_link_stream() {
        let functions = json!({ "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "enUS" }] } });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn skips_items_under_a_non_en_us_locale() {
        let functions = json!({
            "GetLocale": { "player": [{ "t": 0, "tp": 0, "v": "deDE" }] },
            "GetLootSlotLink": { "1": [{ "t": 0, "tp": 0, "v": "|Hitem:750::::::::1::::::::::|h[Zähes Wolfsfleisch]|h[|r" }] },
        });
        assert_eq!(observed(functions), []);
    }
}
