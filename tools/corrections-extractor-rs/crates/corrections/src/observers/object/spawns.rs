//! objectKeys.spawns - Questie field 4 (`{[zoneID] = {{x, y}, ...}, ...}`). Port of
//! `extract/observers/object/spawns.ts`.
//!
//! The player's zone and map position at each object encounter (mostly `target`, when the
//! player interacts with a chest or a node): one observation per spawn point seen, tagged
//! with the unit token it came from. The merge is spawns_merge.rs.

use crate::observation::{Confidence, Observation};
use crate::observers::object::encounters::object_encounters;
use crate::observers::spawn::{SpawnObservation, SpawnToken};
use crate::probes::{player_pos_at, player_zone_at, to_questie_map_position};
use trace_data::Session;

pub fn observe(session: &Session) -> Vec<Observation<SpawnObservation>> {
    let mut observations = Vec::new();
    for encounter in object_encounters(session) {
        let Some(zone_id) = player_zone_at(session, encounter.t) else { continue };
        let Some(pos) = player_pos_at(session, encounter.t) else { continue };
        let questie_pos = to_questie_map_position(pos);
        let token = SpawnToken::from_unit_token(encounter.token).expect("every tracked unit token is a spawn token");
        let value = SpawnObservation { zone_id, x: questie_pos.x, y: questie_pos.y, token };
        observations.push(Observation::new(encounter.object_id, value, Confidence::High, encounter.t));
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

    #[test]
    fn converts_normalized_map_coordinates_to_questie_percentages() {
        let functions = json!({
            "UnitGUID": { "target": [{ "t": 10, "tp": 10, "v": "GameObject-0-5208-0-7-2843-0000399" }] },
            // uiMapID 1436 is Westfall, area 40.
            "C_Map.GetBestMapForUnit": { "player": [{ "t": 10, "tp": 10, "v": 1436 }] },
            "C_Map.GetPlayerMapPosition": { "player": [{ "t": 10, "tp": 10, "v": { "x": 0.5611, "y": 0.6164 } }] },
        });
        let expected = SpawnObservation { zone_id: 40.0, x: 56.11, y: 61.64, token: SpawnToken::Target };
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
}
