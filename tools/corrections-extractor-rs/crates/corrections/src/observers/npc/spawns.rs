//! npcKeys.spawns - Questie field 7 (`{[zoneID] = {{x, y}, ...}, ...}`). Port of
//! `extract/observers/npc/spawns.ts`.
//!
//! The player's zone and map position at each NPC encounter: one observation per spawn point
//! seen, tagged with the unit token it came from. The `questnpc` token is the best source: it
//! fires on quest interactions, which need the player next to the NPC. `npc` and `target` can
//! see the NPC from a distance. The merge (spawns_merge.rs) ranks positions by token.

use crate::observation::{Confidence, Observation};
use crate::observers::npc::encounters::npc_encounters;
use crate::observers::spawn::{SpawnObservation, SpawnToken};
use crate::probes::{player_pos_at, player_zone_at, to_questie_map_position};
use trace_data::Session;

pub fn observe(session: &Session) -> Vec<Observation<SpawnObservation>> {
    let mut observations = Vec::new();
    for encounter in npc_encounters(session) {
        let Some(zone_id) = player_zone_at(session, encounter.t) else { continue };
        let Some(pos) = player_pos_at(session, encounter.t) else { continue };
        let questie_pos = to_questie_map_position(pos);
        let token = SpawnToken::from_unit_token(encounter.token).expect("every tracked unit token is a spawn token");
        let value = SpawnObservation { zone_id, x: questie_pos.x, y: questie_pos.y, token };
        observations.push(Observation::new(encounter.npc_id, value, Confidence::High, encounter.t));
    }
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::json;

    fn observed(functions: serde_json::Value) -> Vec<(i64, SpawnObservation, Confidence, f64)> {
        with_session(&session_json(functions, vec![]), |s| observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect())
    }

    fn spawn(zone_id: f64, x: f64, y: f64, token: SpawnToken) -> SpawnObservation {
        SpawnObservation { zone_id, x, y, token }
    }

    #[test]
    fn converts_normalized_map_coordinates_to_questie_percentages() {
        let functions = json!({
            "UnitGUID": { "npc": [{ "t": 10, "tp": 10, "v": "Creature-0-5208-0-7-823-000031" }] },
            // uiMapID 1436 is Westfall, area 40.
            "C_Map.GetBestMapForUnit": { "player": [{ "t": 10, "tp": 10, "v": 1436 }] },
            "C_Map.GetPlayerMapPosition": { "player": [{ "t": 10, "tp": 10, "v": { "x": 0.5611, "y": 0.6164 } }] },
        });
        assert_eq!(observed(functions), [(823, spawn(40.0, 56.11, 61.64, SpawnToken::Npc), Confidence::High, 10.0)]);
    }

    #[test]
    fn skips_encounters_where_the_player_zone_is_unknown() {
        let functions = json!({
            "UnitGUID": { "npc": [{ "t": 10, "tp": 10, "v": "Creature-0-5208-0-7-823-000031" }] },
            "C_Map.GetBestMapForUnit": { "player": [] },
            "C_Map.GetPlayerMapPosition": { "player": [{ "t": 10, "tp": 10, "v": { "x": 0.3001, "y": 0.8602 } }] },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn skips_encounters_where_the_player_position_is_unknown() {
        let functions = json!({
            "UnitGUID": { "npc": [{ "t": 10, "tp": 10, "v": "Creature-0-5208-0-7-823-000031" }] },
            "C_Map.GetBestMapForUnit": { "player": [{ "t": 10, "tp": 10, "v": 1436 }] },
            "C_Map.GetPlayerMapPosition": { "player": [] },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn emits_one_observation_per_zone_the_npc_was_seen_in() {
        let functions = json!({
            "UnitGUID": { "npc": [
                { "t": 10, "tp": 10, "v": "Creature-0-5208-0-7-823-000031" },
                { "t": 20, "tp": 20, "v": "Creature-0-5208-0-7-823-000031" },
            ]},
            // Westfall (area 40), then Elwynn Forest (area 12).
            "C_Map.GetBestMapForUnit": { "player": [{ "t": 10, "tp": 10, "v": 1436 }, { "t": 20, "tp": 20, "v": 1429 }] },
            "C_Map.GetPlayerMapPosition": { "player": [
                { "t": 10, "tp": 10, "v": { "x": 0.3001, "y": 0.8602 } },
                { "t": 20, "tp": 20, "v": { "x": 0.4746, "y": 0.6218 } },
            ]},
        });
        assert_eq!(
            observed(functions),
            [
                (823, spawn(40.0, 30.01, 86.02, SpawnToken::Npc), Confidence::High, 10.0),
                (823, spawn(12.0, 47.46, 62.18, SpawnToken::Npc), Confidence::High, 20.0),
            ]
        );
    }

    #[test]
    fn returns_nothing_without_npc_encounters() {
        let functions = json!({ "UnitGUID": {}, "C_Map.GetBestMapForUnit": {}, "C_Map.GetPlayerMapPosition": {} });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn uses_questnpc_token_encounters_too() {
        let functions = json!({
            "UnitGUID": { "questnpc": [{ "t": 5, "tp": 5, "v": "Creature-0-5208-0-7-197-000032" }] },
            // Elwynn Forest, area 12.
            "C_Map.GetBestMapForUnit": { "player": [{ "t": 5, "tp": 5, "v": 1429 }] },
            "C_Map.GetPlayerMapPosition": { "player": [{ "t": 5, "tp": 5, "v": { "x": 0.4746, "y": 0.6218 } }] },
        });
        assert_eq!(observed(functions), [(197, spawn(12.0, 47.46, 62.18, SpawnToken::QuestNpc), Confidence::High, 5.0)]);
    }
}
