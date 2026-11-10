//! Quest turn-in locations from C_QuestLog.GetQuestsOnMap (QuestPOI tracker). Port of
//! `extract/observers/quest/turnInPoi.ts`.
//!
//! Once a quest in the log is complete, its POI points at the turn-in location. These are no
//! Questie quest field: at finalize they join the spawns of the quest's finisher (see
//! [`super::finished_by`]) with the [`SpawnToken::QuestPoi`] token, which overrides every other
//! spawn source of that NPC or object.
//!
//! Only POIs reported on their own map (poi.mapID == the queried uiMapID) are used, so the
//! coordinates are relative to the zone they are keyed by.

use super::finished_by::FinishedByValue;
use crate::aggregate::Num;
use crate::js;
use crate::observation::{Confidence, Observation};
use crate::observers::spawn::{SpawnObservation, SpawnToken};
use crate::probes::{stream_value_at, to_questie_map_position, MapPosition};
use crate::resources::area_id_for_ui_map_id;
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use trace_data::stream::{entry_t, entry_v};
use trace_data::{KeyRef, Session, Value};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TurnInPoiValue {
    /// AreaTable areaID.
    pub zone_id: f64,
    /// Questie map coordinates, 0-100.
    pub x: f64,
    pub y: f64,
}

pub fn observe(session: &Session) -> Vec<Observation<TurnInPoiValue>> {
    let mut observations = Vec::new();
    let Some(by_map) = session.function("C_QuestLog.GetQuestsOnMap").and_then(Value::as_object) else {
        return observations;
    };

    // `${questID}:${zoneID}:${x},${y}`: numbers compare like their String() forms.
    let mut seen: HashSet<(Num, Num, Num, Num)> = HashSet::new();
    for (map_key, map_stream) in by_map.entries() {
        let ui_map_id = js::number(&map_key.to_string());
        let Some(zone_id) = area_id_for_ui_map_id(Some(ui_map_id)) else { continue };

        // POI updates can lag behind IsComplete, so a POI equal to the last one seen while the
        // quest was incomplete is still the objective location.
        let mut last_incomplete_pos: HashMap<Num, (Num, Num)> = HashMap::new();

        for entry in map_stream.as_array().unwrap_or(&[]) {
            let Some(pois) = entry_v(entry).and_then(Value::as_array) else { continue };
            let t = entry_t(entry);

            for poi in pois {
                let number = |key: &str| poi.get(key).and_then(Value::as_num);
                let (Some(quest_id), Some(x), Some(y)) = (number("questID"), number("x"), number("y")) else {
                    continue;
                };
                if number("mapID") != Some(ui_map_id) || poi.get("isQuestStart").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                if x <= 0.0 || y <= 0.0 {
                    continue;
                }

                let raw_pos = (Num(x), Num(y));
                let is_complete = stream_value_at(session, "C_QuestLog.IsComplete", &[KeyRef::Num(quest_id)], t).and_then(Value::as_bool);
                if is_complete != Some(true) {
                    last_incomplete_pos.insert(Num(quest_id), raw_pos);
                    continue;
                }
                if last_incomplete_pos.get(&Num(quest_id)) == Some(&raw_pos) {
                    continue;
                }

                let pos = to_questie_map_position(MapPosition { x, y });
                if !seen.insert((Num(quest_id), Num(zone_id), Num(pos.x), Num(pos.y))) {
                    continue;
                }

                observations.push(Observation::new(
                    quest_id as i64,
                    TurnInPoiValue { zone_id, x: pos.x, y: pos.y },
                    Confidence::High,
                    t,
                ));
            }
        }
    }

    observations
}

/// Turn-in POIs as spawn observations of each quest's finisher (TS `TurnInSpawnObservation`s).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct TurnInPoiSpawns {
    pub npcs: Vec<Observation<SpawnObservation>>,
    pub objects: Vec<Observation<SpawnObservation>>,
}

/// Converts quest turn-in POIs into spawn observations of the quest's single finisher. Quests
/// with no or several finishers are skipped: their POI cannot be attributed to one of them.
pub fn turn_in_poi_spawns(pois: &[Observation<TurnInPoiValue>], finished_by: &IndexMap<i64, FinishedByValue>) -> TurnInPoiSpawns {
    let mut spawns = TurnInPoiSpawns::default();

    for poi in pois {
        let Some(finisher) = finished_by.get(&poi.entity_id) else {
            continue;
        };
        let (target, finisher_id) = match (finisher.creatures.as_slice(), finisher.objects.as_slice()) {
            (&[npc_id], []) => (&mut spawns.npcs, npc_id),
            ([], &[object_id]) => (&mut spawns.objects, object_id),
            _ => continue,
        };
        let TurnInPoiValue { zone_id, x, y } = poi.value;
        target.push(Observation {
            entity_id: finisher_id,
            value: SpawnObservation {
                zone_id,
                x,
                y,
                token: SpawnToken::QuestPoi,
            },
            confidence: poi.confidence,
            t: poi.t,
            session: poi.session,
        });
    }

    spawns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, stream, with_session};
    use serde_json::{json, Value as Json};

    const BARRENS_UI_MAP: u32 = 1413;
    const BARRENS_AREA: f64 = 17.0;

    fn poi(quest_id: u32, map_id: u32, x: f64, y: f64) -> Json {
        json!({ "questID": quest_id, "mapID": map_id, "x": x, "y": y, "isQuestStart": false, "inProgress": true })
    }

    /// `C_QuestLog.GetQuestsOnMap` keyed by map, plus `C_QuestLog.IsComplete` for quest 891.
    fn functions(quests_on_map: Json, is_complete: &[(f64, bool)]) -> Json {
        json!({
            "C_QuestLog.GetQuestsOnMap": quests_on_map,
            "C_QuestLog.IsComplete": { "891": stream(is_complete.iter().map(|&(t, v)| (t, json!(v)))) },
        })
    }

    fn observed(functions: Json) -> Vec<(i64, TurnInPoiValue, Confidence, f64)> {
        with_session(&session_json(functions, vec![]), |s| {
            observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect()
        })
    }

    fn on_barrens(entries: impl IntoIterator<Item = (f64, Json)>) -> Json {
        json!({ BARRENS_UI_MAP.to_string(): stream(entries) })
    }

    #[test]
    fn observes_the_poi_position_of_a_complete_quest() {
        let quests_on_map = on_barrens([(5.0, json!([poi(891, BARRENS_UI_MAP, 0.6228, 0.3903)]))]);
        assert_eq!(
            observed(functions(quests_on_map, &[(0.0, true)])),
            [(
                891,
                TurnInPoiValue {
                    zone_id: BARRENS_AREA,
                    x: 62.28,
                    y: 39.03
                },
                Confidence::High,
                5.0
            )]
        );
    }

    #[test]
    fn ignores_pois_of_incomplete_quests() {
        let quests_on_map = on_barrens([(5.0, json!([poi(891, BARRENS_UI_MAP, 0.62, 0.39)]))]);
        assert_eq!(observed(functions(quests_on_map, &[(0.0, false), (10.0, true)])), []);
    }

    #[test]
    fn ignores_pois_reported_on_another_map_than_queried() {
        let quests_on_map = on_barrens([(5.0, json!([poi(891, 1414, 0.62, 0.39)]))]);
        assert_eq!(observed(functions(quests_on_map, &[(0.0, true)])), []);
    }

    #[test]
    fn ignores_a_lagging_poi_still_showing_the_objective_location_after_completion() {
        let quests_on_map = on_barrens([
            (5.0, json!([poi(891, BARRENS_UI_MAP, 0.3, 0.3)])),
            (10.0, json!([poi(891, BARRENS_UI_MAP, 0.3, 0.3), poi(1, BARRENS_UI_MAP, 0.1, 0.1)])),
            (10.35, json!([poi(891, BARRENS_UI_MAP, 0.62, 0.39)])),
        ]);
        let values: Vec<TurnInPoiValue> = observed(functions(quests_on_map, &[(0.0, false), (10.0, true)]))
            .into_iter()
            .map(|o| o.1)
            .collect();
        assert_eq!(
            values,
            [TurnInPoiValue {
                zone_id: BARRENS_AREA,
                x: 62.0,
                y: 39.0
            }]
        );
    }

    #[test]
    fn ignores_pois_on_maps_without_a_questie_zone() {
        let quests_on_map = json!({ "1414": stream([(5.0, json!([poi(891, 1414, 0.62, 0.39)]))]) });
        assert_eq!(observed(functions(quests_on_map, &[(0.0, true)])), []);
    }

    #[test]
    fn ignores_quest_start_pois() {
        let mut start = poi(891, BARRENS_UI_MAP, 0.62, 0.39);
        start["isQuestStart"] = json!(true);
        let quests_on_map = on_barrens([(5.0, json!([start]))]);
        assert_eq!(observed(functions(quests_on_map, &[(0.0, true)])), []);
    }

    #[test]
    fn deduplicates_repeated_samples_of_the_same_poi() {
        let quests_on_map = on_barrens([
            (5.0, json!([poi(891, BARRENS_UI_MAP, 0.62, 0.39)])),
            (6.0, json!([poi(891, BARRENS_UI_MAP, 0.62, 0.39), poi(1, BARRENS_UI_MAP, 0.1, 0.1)])),
        ]);
        assert_eq!(observed(functions(quests_on_map, &[(0.0, true)])).len(), 1);
    }

    // describe("turnInPoiSpawns")

    fn poi_obs() -> Observation<TurnInPoiValue> {
        Observation::new(
            891,
            TurnInPoiValue {
                zone_id: BARRENS_AREA,
                x: 62.28,
                y: 39.03,
            },
            Confidence::High,
            5.0,
        )
    }

    fn finished_by(quest_id: i64, creatures: &[i64], objects: &[i64]) -> IndexMap<i64, FinishedByValue> {
        let value = FinishedByValue {
            creatures: creatures.to_vec(),
            objects: objects.to_vec(),
        };
        IndexMap::from([(quest_id, value)])
    }

    fn spawn_of(entity_id: i64) -> Observation<SpawnObservation> {
        let poi = poi_obs();
        Observation {
            entity_id,
            value: SpawnObservation {
                zone_id: BARRENS_AREA,
                x: 62.28,
                y: 39.03,
                token: SpawnToken::QuestPoi,
            },
            confidence: poi.confidence,
            t: poi.t,
            session: poi.session,
        }
    }

    #[test]
    fn attributes_the_poi_to_a_single_creature_finisher() {
        let spawns = turn_in_poi_spawns(&[poi_obs()], &finished_by(891, &[3453], &[]));
        assert_eq!(
            spawns,
            TurnInPoiSpawns {
                npcs: vec![spawn_of(3453)],
                objects: vec![]
            }
        );
    }

    #[test]
    fn attributes_the_poi_to_a_single_object_finisher() {
        let spawns = turn_in_poi_spawns(&[poi_obs()], &finished_by(891, &[], &[9000]));
        assert_eq!(
            spawns,
            TurnInPoiSpawns {
                npcs: vec![],
                objects: vec![spawn_of(9000)]
            }
        );
    }

    #[test]
    fn skips_quests_with_ambiguous_or_unknown_finishers() {
        assert_eq!(turn_in_poi_spawns(&[poi_obs()], &finished_by(891, &[1, 2], &[])), TurnInPoiSpawns::default());
        assert_eq!(turn_in_poi_spawns(&[poi_obs()], &IndexMap::new()), TurnInPoiSpawns::default());
    }
}
