//! objectKeys.spawns - Questie field 4 (`{[zoneID] = {{x, y}, ...}, ...}`). Port of
//! `extract/observers/object/spawns.ts`.
//!
//! The player's zone and map position at each object encounter (mostly `target`, when the
//! player interacts with a chest or a node) and at each loot of an object (`loot`): one
//! observation per spawn point seen, tagged with the token it came from. The merge is
//! spawns_merge.rs.

use crate::observation::{Confidence, Observation};
use crate::observers::object::encounters::object_encounters;
use crate::observers::object::loots::object_loots;
use crate::observers::spawn::{SpawnObservation, SpawnToken};
use crate::probes::{player_pos_at, player_zone_at, to_questie_map_position};
use trace_data::Session;

pub fn observe(session: &Session) -> Vec<Observation<SpawnObservation>> {
    let encounters = object_encounters(session).into_iter().map(|encounter| {
        let token = SpawnToken::from_unit_token(encounter.token).expect("every tracked unit token is a spawn token");
        (encounter.object_id, encounter.t, token)
    });
    // Looted objects (chests, crates, ...) are mostly never targeted, so their loots are a source of their own.
    let loots = object_loots(session).into_iter().map(|loot| (loot.object_id, loot.t, SpawnToken::Loot));

    let mut observations = Vec::new();
    for (object_id, t, token) in encounters.chain(loots) {
        let Some(zone_id) = player_zone_at(session, t) else { continue };
        let Some(pos) = player_pos_at(session, t) else { continue };
        let questie_pos = to_questie_map_position(pos);
        let value = SpawnObservation {
            zone_id,
            x: questie_pos.x,
            y: questie_pos.y,
            token,
        };
        observations.push(Observation::new(object_id, value, Confidence::High, t));
    }
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{event, session_json, with_session};
    use serde_json::json;

    fn observed(functions: serde_json::Value) -> Vec<(i64, SpawnObservation, Confidence, f64)> {
        observed_with_events(functions, vec![])
    }

    fn observed_with_events(functions: serde_json::Value, events: Vec<serde_json::Value>) -> Vec<(i64, SpawnObservation, Confidence, f64)> {
        with_session(&session_json(functions, events), |s| {
            observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect()
        })
    }

    #[test]
    fn converts_normalized_map_coordinates_to_questie_percentages() {
        let functions = json!({
            "UnitGUID": { "target": [{ "t": 10, "tp": 10, "v": "GameObject-0-5208-0-7-2843-0000399" }] },
            // uiMapID 1436 is Westfall, area 40.
            "C_Map.GetBestMapForUnit": { "player": [{ "t": 10, "tp": 10, "v": 1436 }] },
            "C_Map.GetPlayerMapPosition": { "player": [{ "t": 10, "tp": 10, "v": { "x": 0.5611, "y": 0.6164 } }] },
        });
        let expected = SpawnObservation {
            zone_id: 40.0,
            x: 56.11,
            y: 61.64,
            token: SpawnToken::Target,
        };
        assert_eq!(observed(functions), [(2843, expected, Confidence::High, 10.0)]);
    }

    #[test]
    fn skips_encounters_where_the_player_zone_is_unknown() {
        let functions = json!({
            "UnitGUID": { "target": [{ "t": 10, "tp": 10, "v": "GameObject-0-5208-0-7-2843-0000399" }] },
            "C_Map.GetBestMapForUnit": { "player": [] },
            "C_Map.GetPlayerMapPosition": { "player": [{ "t": 10, "tp": 10, "v": { "x": 0.3001, "y": 0.8602 } }] },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn skips_encounters_where_the_player_position_is_unknown() {
        let functions = json!({
            "UnitGUID": { "target": [{ "t": 10, "tp": 10, "v": "GameObject-0-5208-0-7-2843-0000399" }] },
            "C_Map.GetBestMapForUnit": { "player": [{ "t": 10, "tp": 10, "v": 1436 }] },
            "C_Map.GetPlayerMapPosition": { "player": [] },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn returns_nothing_without_object_encounters() {
        let functions = json!({ "UnitGUID": {}, "C_Map.GetBestMapForUnit": {}, "C_Map.GetPlayerMapPosition": {} });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn observes_a_looted_objects_spawn_at_the_players_position_with_the_loot_token() {
        let functions = json!({
            "GetLootSourceInfo": { "1": [{ "t": 5, "tp": 5, "v": { "1": "GameObject-0-5208-0-7-673206-0000399", "2": 1, "n": 2 } }] },
            // uiMapID 1436 is Westfall, area 40.
            "C_Map.GetBestMapForUnit": { "player": [{ "t": 0, "tp": 0, "v": 1436 }] },
            "C_Map.GetPlayerMapPosition": { "player": [{ "t": 0, "tp": 0, "v": { "x": 0.5611, "y": 0.6164 } }] },
        });
        let expected = SpawnObservation {
            zone_id: 40.0,
            x: 56.11,
            y: 61.64,
            token: SpawnToken::Loot,
        };
        let events = vec![event(5.0, "LOOT_READY", json!({ "n": 0 }))];
        assert_eq!(observed_with_events(functions, events), [(673206, expected, Confidence::High, 5.0)]);
    }
}
