//! npcKeys and objectKeys zoneID and spawns of npcs and objects that live in an instance. Not in
//! the TypeScript extractor.
//!
//! Inside an instance the client gives no player position, so a position cannot be observed there.
//! A creature's or object's GUID ("Creature-0-<server>-<instance map>-<zone uid>-<id>-<spawn>") does
//! name the map it is in. Source: the GUIDs of GetLootSourceInfo (Modules/Trackers/Loot.lua).
//!
//! The merge gives a zone to an entity only ever seen on one map, so one that also lives elsewhere
//! (ore and herb nodes, escorted npcs) gets none. The map is not in QuestieDB's table of instances,
//! or is the open world: no zone.
//!
//! Checked against the Classic npc and object data: 738 of 738 entities seen on one map agree, as do
//! all 35 hand-written instance zones that were seen. Those entities are known to QuestieDB; the ones
//! this exports are not, so nothing confirms them one by one.

use crate::aggregate::Num;
use crate::guid::GuidKind;
use crate::observation::{Confidence, Observation};
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

/// QuestieDB's instance map -> area id (support/Forever/Zones/instanceIdToAreaId.lua, 4377c98).
#[rustfmt::skip]
const AREA_OF_INSTANCE: &[(i64, i64)] = &[
    (30, 2597),
    (33, 209),
    (34, 717),
    (36, 1581),
    (43, 718),
    (47, 491),
    (48, 719),
    (70, 1337),
    (90, 721),
    (109, 1477),
    (129, 722),
    (189, 796),
    (209, 1176),
    (229, 1583),
    (230, 1584),
    (249, 2159),
    (269, 2366),
    (289, 2057),
    (309, 1977),
    (329, 2017),
    (349, 2100),
    (369, 2257),
    (389, 2437),
    (409, 2717),
    (429, 2557),
    (449, 2918),
    (450, 2917),
    (469, 2677),
    (489, 3277),
    (509, 3429),
    (529, 3358),
    (531, 3428),
    (532, 3457),
    (533, 3456),
    (534, 3606),
    (540, 3714),
    (542, 3713),
    (543, 3562),
    (544, 3836),
    (545, 3715),
    (546, 3716),
    (547, 3717),
    (548, 3607),
    (550, 3845),
    (552, 3848),
    (553, 3847),
    (554, 3849),
    (555, 3789),
    (556, 3791),
    (557, 3792),
    (558, 3790),
    (560, 2367),
    (564, 3959),
    (565, 3923),
    (568, 3805),
    (574, 206),
    (575, 1196),
    (576, 4265),
    (578, 4228),
    (580, 4075),
    (585, 4131),
    (595, 4100),
    (599, 4264),
    (600, 4196),
    (601, 4277),
    (602, 4272),
    (603, 4273),
    (604, 4416),
    (608, 4415),
    (615, 4493),
    (616, 4500),
    (619, 4494),
    (631, 4812),
    (632, 4809),
    (643, 5004),
    (644, 4945),
    (645, 4926),
    (649, 4722),
    (650, 4723),
    (657, 5035),
    (658, 4813),
    (668, 4820),
    (669, 5094),
    (670, 4950),
    (671, 5334),
    (720, 5723),
    (724, 4987),
    (725, 5088),
    (754, 5638),
    (755, 5396),
    (859, 1977),
    (938, 5789),
    (939, 5788),
    (940, 5844),
    (959, 5918),
    (960, 5956),
    (961, 5963),
    (962, 5976),
    (967, 5892),
    (994, 6182),
    (996, 6067),
    (1001, 6052),
    (1004, 796),
    (1007, 2057),
    (1008, 6125),
    (1009, 6297),
    (1011, 6214),
    (1098, 6622),
    (1136, 6738),
    (2784, 15475),
    (2789, 15531),
    (2791, 15532),
    (2804, 15825),
    (2807, 15828),
    (2817, 15938),
    (2856, 16236),
    (2875, 16074),
    (2902, 16295),
    (2921, 16394),
    (2959, 16544),
    (2997, 16606),
    (2998, 16732),
    (2999, 16611),
    (3005, 16653),
    (3065, 16919),
    (3109, 17191),
];

/// The area id of an instance map; None for the open world and unknown instances.
pub fn area_of_instance(map: f64) -> Option<i64> {
    AREA_OF_INSTANCE.iter().find(|(instance, _)| *instance as f64 == map).map(|&(_, area)| area)
}

/// Every map the loot sources of `kind` were on, one observation per source and loot sample.
pub fn observe(session: &Session, kind: GuidKind) -> Vec<Observation<Num>> {
    let Some(root) = session.function("GetLootSourceInfo") else {
        return Vec::new();
    };
    let mut observations = Vec::new();
    for slot in param_keys(root) {
        for entry in get_stream(session, "GetLootSourceInfo", &[KeyRef::Str(&slot)]).unwrap_or_default() {
            let Some(sources) = entry_v(entry).filter(|value| value.is_table()) else {
                continue;
            };
            let count = sources.get("n").and_then(Value::as_num).unwrap_or(0.0) as u32;
            // guid1, quantity1, guid2, quantity2, ...
            for index in (1..=count).step_by(2) {
                let Some((id, map)) = sources.get(index).and_then(Value::as_str).and_then(|guid| entity_and_map(guid, kind)) else {
                    continue;
                };
                observations.push(Observation::new(id, Num(map), Confidence::High, entry_t(entry)));
            }
        }
    }
    observations
}

/// The id and map of a creature, vehicle or object GUID of `kind`.
fn entity_and_map(guid: &str, kind: GuidKind) -> Option<(i64, f64)> {
    let parts: Vec<&str> = guid.split('-').collect();
    let [guid_type, _, _, map, _, id, _] = parts[..] else { return None };
    let guid_kind = match guid_type {
        "Creature" | "Vehicle" => GuidKind::Npc,
        "GameObject" => GuidKind::Object,
        _ => return None,
    };
    if guid_kind != kind {
        return None;
    }
    Some((id.parse().ok()?, map.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{packed, session_json, with_session};
    use serde_json::{json, Value as Json};

    fn loot(sources: &[&str]) -> Json {
        let values: Vec<Json> = sources.iter().flat_map(|guid| [json!(guid), json!(1)]).collect();
        json!({ "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": packed(&values) }] } })
    }

    fn observed(functions: Json, kind: GuidKind) -> Vec<(i64, f64)> {
        with_session(&session_json(functions, vec![]), |s| {
            observe(s, kind).into_iter().map(|o| (o.entity_id, o.value.0)).collect()
        })
    }

    const DEADMINES_NPC: &str = "Creature-0-5208-36-7-30001-000031";
    const DEADMINES_CHEST: &str = "GameObject-0-5208-36-7-300200-0000399";

    #[test]
    fn observes_the_map_of_every_loot_source_of_the_kind() {
        let functions = loot(&[DEADMINES_NPC, "Vehicle-0-5208-33-7-30002-000032", DEADMINES_CHEST]);
        assert_eq!(observed(functions.clone(), GuidKind::Npc), [(30001, 36.0), (30002, 33.0)]);
        assert_eq!(observed(functions, GuidKind::Object), [(300200, 36.0)]);
    }

    #[test]
    fn reads_guids_only_from_the_guid_positions_and_never_a_players() {
        let functions = loot(&["Player-5284-0362", "Pet-0-5208-36-7-30003-000033", "not a guid", "Creature-0-36-7-30004-1"]);
        assert_eq!(observed(functions.clone(), GuidKind::Npc), []);
        assert_eq!(observed(json!({}), GuidKind::Npc), []);
    }

    #[test]
    fn knows_the_area_of_an_instance_and_no_area_for_the_open_world() {
        assert_eq!(area_of_instance(36.0), Some(1581), "The Deadmines");
        assert_eq!(area_of_instance(2959.0), Some(16544), "City of Dalaran");
        assert_eq!(area_of_instance(0.0), None);
        assert_eq!(area_of_instance(1.0), None);
        assert_eq!(area_of_instance(2991.0), None, "not in QuestieDB's table");
    }
}
