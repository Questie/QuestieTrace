//! itemKeys.objectDrops. Port of `extract/observers/item/objectDrops.ts`.
//!
//! Like npc_drops.rs, for loot sources that are game objects: one observation per object a
//! looted item came from. The merge (drops_merge.rs) unions the object ids per item.

use crate::guid::GuidKind;
use crate::observation::{Confidence, Observation};
use crate::observers::item::npc_drops::loot_sources;
use trace_data::Session;

/// One object an item was looted from (TS `{ objectID }`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ObjectDrop {
    pub object_id: i64,
}

pub fn observe(session: &Session) -> Vec<Observation<ObjectDrop>> {
    loot_sources(session, GuidKind::Object)
        .into_iter()
        .map(|source| Observation::new(source.item_id, ObjectDrop { object_id: source.source_id }, Confidence::High, source.t))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::json;

    fn observed(functions: serde_json::Value) -> Vec<(i64, i64, Confidence, f64)> {
        with_session(&session_json(functions, vec![]), |s| observe(s).into_iter().map(|o| (o.entity_id, o.value.object_id, o.confidence, o.t)).collect())
    }

    #[test]
    fn pairs_the_loot_source_object_with_the_slot_link_at_the_same_time() {
        let functions = json!({
            "GetLootSlotLink": { "1": [{ "t": 5, "tp": 5, "v": "|Hitem:12345::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }] },
            "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": { "1": "GameObject-0-5208-0-7-2843-0000399", "2": 1, "n": 2 } }] },
        });
        assert_eq!(observed(functions), [(12345, 2843, Confidence::High, 5.0)]);
    }

    #[test]
    fn reads_every_object_of_a_multi_source_tuple() {
        let functions = json!({
            "GetLootSlotLink": { "1": [{ "t": 5, "tp": 5, "v": "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }] },
            "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": {
                "1": "GameObject-0-5208-0-7-2843-0000399", "2": 1, "3": "GameObject-0-5208-0-7-2844-0000400", "4": 1, "n": 4,
            } }] },
        });
        assert_eq!(observed(functions), [(12345, 2843, Confidence::High, 5.0), (12345, 2844, Confidence::High, 5.0)]);
    }

    #[test]
    fn skips_npc_loot_sources() {
        let functions = json!({
            "GetLootSlotLink": { "1": [{ "t": 5, "tp": 5, "v": "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }] },
            "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": { "1": "Creature-0-5208-0-7-823-000031", "2": 1, "n": 2 } }] },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn skips_items_without_a_loot_source_stream() {
        let functions = json!({ "GetLootSlotLink": { "1": [{ "t": 5, "tp": 5, "v": "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }] } });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn walks_every_loot_time() {
        let functions = json!({
            "GetLootSlotLink": { "1": [
                { "t": 5, "tp": 5, "v": "|Hitem:111::::::::1::::::::::|h[Feather]|h[|r" },
                { "t": 10, "tp": 10, "v": "|Hitem:222::::::::1::::::::::|h[Bone]|h[|r" },
            ]},
            "GetLootSourceInfo": { "1": [
                { "t": 5, "tp": 5, "v": { "1": "GameObject-0-5208-0-7-2843-0000399", "2": 1, "n": 2 } },
                { "t": 10, "tp": 10, "v": { "1": "GameObject-0-5208-0-7-2844-0000400", "2": 1, "n": 2 } },
            ]},
        });
        assert_eq!(observed(functions), [(111, 2843, Confidence::High, 5.0), (222, 2844, Confidence::High, 10.0)]);
    }

    #[test]
    fn returns_nothing_without_loot_data() {
        assert_eq!(observed(json!({})), []);
    }
}
