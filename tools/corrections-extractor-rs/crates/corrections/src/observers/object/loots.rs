//! The objects the player looted: one entry per loot of a game object, read from the
//! GetLootSourceInfo(slot) streams (Modules/Trackers/Loot.lua). Looting needs the player right
//! next to the object, so the player's position then is the object's position. Port of
//! `extract/observers/object/_loots.ts`.
//!
//! Privacy: only object GUIDs are read; the Loot tracker drops player GUIDs before recording.

use crate::aggregate::Num;
use crate::guid::{parse_guid, GuidKind};
use indexmap::IndexMap;
use std::cmp::Ordering;
use std::collections::HashSet;
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ObjectLoot {
    pub object_id: i64,
    pub t: f64,
}

/// Every loot of an object, by time and then objectID.
pub fn object_loots(session: &Session) -> Vec<ObjectLoot> {
    let Some(sources) = session.function("GetLootSourceInfo") else {
        return Vec::new();
    };
    // Slots are probed again at LOOT_CLOSED, after the loot: only the samples of an opening are
    // loots. Num compares times the way a JS Set does.
    let opened_at: HashSet<Num> = session.events().filter(|event| event.is("LOOT_READY")).map(|event| Num(event.t())).collect();

    // A loot fills several slots from the same source: count it once (TS keys `${t}:${guid}`).
    let mut loots: IndexMap<(Num, &str), ObjectLoot> = IndexMap::new();
    for slot in param_keys(sources) {
        for entry in get_stream(session, "GetLootSourceInfo", &[KeyRef::Str(&slot)]).unwrap_or_default() {
            let t = entry_t(entry);
            if !opened_at.contains(&Num(t)) {
                continue;
            }
            let Some(source) = entry_v(entry).filter(|v| v.is_table()) else { continue };
            // Packed (guid1, quantity1, guid2, quantity2, ...).
            let count = source.get("n").and_then(Value::as_num).unwrap_or(0.0);
            let mut i = 1.0;
            while i <= count {
                if let Some(guid) = source.get(i).and_then(Value::as_str) {
                    let parsed = parse_guid(Some(guid));
                    if let (GuidKind::Object, Some(id)) = (parsed.kind, parsed.id) {
                        loots.insert((Num(t), guid), ObjectLoot { object_id: id as i64, t });
                    }
                }
                i += 2.0;
            }
        }
    }
    let mut loots: Vec<ObjectLoot> = loots.into_values().collect();
    // `a.t - b.t || a.objectID - b.objectID`: equal or incomparable times fall to the objectID.
    loots.sort_by(|a, b| match a.t.partial_cmp(&b.t) {
        Some(Ordering::Equal) | None => a.object_id.cmp(&b.object_id),
        Some(order) => order,
    });
    loots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{event, session_json, with_session};
    use serde_json::json;

    fn loots(functions: serde_json::Value, loots_opened_at: &[f64]) -> Vec<(i64, f64)> {
        let events = loots_opened_at.iter().map(|&t| event(t, "LOOT_READY", json!({ "n": 0 }))).collect();
        with_session(&session_json(functions, events), |s| {
            object_loots(s).into_iter().map(|loot| (loot.object_id, loot.t)).collect()
        })
    }

    #[test]
    fn counts_a_loot_of_several_slots_from_the_same_object_once() {
        let source = json!({ "1": "GameObject-0-5208-0-7-673206-0000399", "2": 1, "n": 2 });
        let functions = json!({ "GetLootSourceInfo": {
            "1": [{ "t": 5, "tp": 5, "v": source }],
            "2": [{ "t": 5, "tp": 5, "v": source }],
        }});
        assert_eq!(loots(functions, &[5.0]), [(673206, 5.0)]);
    }

    #[test]
    fn does_not_count_the_sample_taken_when_the_loot_window_closes() {
        let source = json!({ "1": "GameObject-0-5208-0-7-673206-0000399", "2": 1, "n": 2 });
        let functions = json!({ "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": source }, { "t": 7, "tp": 7, "v": source }] } });
        assert_eq!(loots(functions, &[5.0]), [(673206, 5.0)]);
    }

    #[test]
    fn ignores_loot_from_npcs_and_empty_slots() {
        let functions = json!({ "GetLootSourceInfo": { "1": [
            { "t": 5, "tp": 5, "v": { "1": "Creature-0-5208-0-7-823-000031", "2": 1, "n": 2 } },
            { "t": 6, "tp": 6 },
        ]}});
        assert_eq!(loots(functions, &[5.0, 6.0]), []);
    }

    #[test]
    fn returns_every_loot_of_the_same_object_at_different_times() {
        let functions = json!({ "GetLootSourceInfo": { "1": [
            { "t": 5, "tp": 5, "v": { "1": "GameObject-0-5208-0-7-673206-0000399", "2": 1, "n": 2 } },
            { "t": 9, "tp": 9, "v": { "1": "GameObject-0-5208-0-7-673206-0000400", "2": 1, "n": 2 } },
        ]}});
        assert_eq!(loots(functions, &[5.0, 9.0]), [(673206, 5.0), (673206, 9.0)]);
    }
}
