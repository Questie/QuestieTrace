//! itemKeys.npcDrops - Questie field 2. Port of `extract/observers/item/npcDrops.ts`.
//!
//! GetLootSourceInfo(slot), sampled by Modules/Trackers/Loot.lua on LOOT_READY/LOOT_CLOSED,
//! paired with GetLootSlotLink(slot) sampled at the same time: one observation per NPC a
//! looted item came from. The merge (drops_merge.rs) unions the NPC ids per item.
//!
//! Privacy: the Loot tracker drops player and unrecognized GUIDs before recording, so only
//! npc, object and item GUIDs are seen here.

use crate::guid::{parse_guid, GuidKind};
use crate::item_link::parse_item_link;
use crate::observation::{Confidence, Observation};
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

/// One NPC an item was looted from (TS `{ npcID }`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NpcDrop {
    pub npc_id: i64,
}

pub fn observe(session: &Session) -> Vec<Observation<NpcDrop>> {
    loot_sources(session, GuidKind::Npc)
        .into_iter()
        .map(|source| Observation::new(source.item_id, NpcDrop { npc_id: source.source_id }, Confidence::High, source.t))
        .collect()
}

/// An item looted from an npc or object at `t`.
pub(crate) struct LootSource {
    pub item_id: i64,
    pub source_id: i64,
    pub t: f64,
}

/// Every (item, source of `kind`) pair of the session: slot by slot, link sample by link
/// sample, the sources in the order GetLootSourceInfo returned them. Shared with
/// object_drops.rs, whose TS twin differs from npcDrops.ts only in the GUID kind.
pub(crate) fn loot_sources(session: &Session, kind: GuidKind) -> Vec<LootSource> {
    let mut sources = Vec::new();
    let (Some(link_stream), Some(_)) = (session.function("GetLootSlotLink"), session.function("GetLootSourceInfo")) else { return sources };
    for slot in param_keys(link_stream) {
        let slot = [KeyRef::Str(&slot)];
        let (Some(link_entries), Some(source_entries)) = (get_stream(session, "GetLootSlotLink", &slot), get_stream(session, "GetLootSourceInfo", &slot)) else {
            continue;
        };
        for link_entry in link_entries {
            let Some(link) = parse_item_link(entry_v(link_entry).and_then(Value::as_str)) else { continue };
            // `e.t === t`: an entry without `t` matches another without one, NaN matches nothing.
            let t = link_entry.get("t").and_then(Value::as_num);
            let Some(source_at) = source_entries.iter().find(|entry| entry.get("t").and_then(Value::as_num) == t) else { continue };
            let Some(source) = entry_v(source_at).filter(|v| v.is_table()) else { continue };
            // Packed (guid1, quantity1, guid2, quantity2, ...).
            let count = source.get("n").and_then(Value::as_num).unwrap_or(0.0);
            let mut i = 1.0;
            while i <= count {
                if let Some(guid) = source.get(i).and_then(Value::as_str) {
                    let parsed = parse_guid(Some(guid));
                    if let (true, Some(id)) = (parsed.kind == kind, parsed.id) {
                        sources.push(LootSource { item_id: link.item_id as i64, source_id: id as i64, t: entry_t(link_entry) });
                    }
                }
                i += 2.0;
            }
        }
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::json;

    fn observed(functions: serde_json::Value) -> Vec<(i64, i64, Confidence, f64)> {
        with_session(&session_json(functions, vec![]), |s| observe(s).into_iter().map(|o| (o.entity_id, o.value.npc_id, o.confidence, o.t)).collect())
    }

    #[test]
    fn pairs_the_loot_source_npc_with_the_slot_link_at_the_same_time() {
        let functions = json!({
            "GetLootSlotLink": { "1": [{ "t": 5, "tp": 5, "v": "|Hitem:12345::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }] },
            "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": { "1": "Creature-0-5208-0-7-823-000031", "2": 1, "n": 2 } }] },
        });
        assert_eq!(observed(functions), [(12345, 823, Confidence::High, 5.0)]);
    }

    #[test]
    fn reads_every_npc_of_a_multi_source_tuple() {
        let functions = json!({
            "GetLootSlotLink": { "1": [{ "t": 5, "tp": 5, "v": "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }] },
            "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": {
                "1": "Creature-0-5208-0-7-823-000031", "2": 1, "3": "Creature-0-5208-0-7-824-000032", "4": 1, "n": 4,
            } }] },
        });
        assert_eq!(observed(functions), [(12345, 823, Confidence::High, 5.0), (12345, 824, Confidence::High, 5.0)]);
    }

    #[test]
    fn skips_object_loot_sources() {
        let functions = json!({
            "GetLootSlotLink": { "1": [{ "t": 5, "tp": 5, "v": "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }] },
            "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": { "1": "GameObject-0-5208-0-7-2843-0000399", "2": 1, "n": 2 } }] },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn skips_items_without_a_loot_source_stream() {
        let functions = json!({ "GetLootSlotLink": { "1": [{ "t": 5, "tp": 5, "v": "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }] } });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn walks_every_slot_and_loot_time() {
        let functions = json!({
            "GetLootSlotLink": {
                "1": [
                    { "t": 5, "tp": 5, "v": "|Hitem:111::::::::1::::::::::|h[Feather]|h[|r" },
                    { "t": 10, "tp": 10, "v": "|Hitem:222::::::::1::::::::::|h[Bone]|h[|r" },
                ],
                "2": [{ "t": 5, "tp": 5, "v": "|Hitem:333::::::::1::::::::::|h[Hide]|h[|r" }],
            },
            "GetLootSourceInfo": {
                "1": [
                    { "t": 5, "tp": 5, "v": { "1": "Creature-0-5208-0-7-823-000031", "2": 1, "n": 2 } },
                    { "t": 10, "tp": 10, "v": { "1": "Creature-0-5208-0-7-824-000032", "2": 1, "n": 2 } },
                ],
                "2": [{ "t": 5, "tp": 5, "v": { "1": "Creature-0-5208-0-7-825-000033", "2": 1, "n": 2 } }],
            },
        });
        assert_eq!(observed(functions), [(111, 823, Confidence::High, 5.0), (222, 824, Confidence::High, 10.0), (333, 825, Confidence::High, 5.0)]);
    }

    #[test]
    fn returns_nothing_without_loot_data() {
        assert_eq!(observed(json!({})), []);
    }

    #[test]
    fn skips_slots_whose_link_and_source_were_sampled_at_different_times() {
        let functions = json!({
            "GetLootSlotLink": { "1": [{ "t": 5, "tp": 5, "v": "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }] },
            "GetLootSourceInfo": { "1": [{ "t": 10, "tp": 10, "v": { "1": "Creature-0-5208-0-7-823-000031", "2": 1, "n": 2 } }] },
        });
        assert_eq!(observed(functions), []);
    }
}
