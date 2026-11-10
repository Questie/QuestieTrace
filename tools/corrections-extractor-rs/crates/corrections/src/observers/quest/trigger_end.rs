//! questKeys.triggerEnd - Questie field 9: `{text, {[zoneID] = {coordPair, ...}}}`. Port of
//! `extract/observers/quest/triggerEnd.ts`.
//!
//! Event objectives are completed by the player in the world rather than by an entity
//! interaction. C_QuestLog.GetQuestObjectives exposes their text and finished state; a
//! completed sample is correlated with the player's map position. Samples repeat the finished
//! state long after the completion, so a location is recorded only on an objective's
//! incomplete->complete transition, or on its first sample when it is first seen finished.

use crate::aggregate::Num;
use crate::js;
use crate::observation::{Confidence, Observation};
use crate::probes::{locale_at, player_pos_at, player_zone_at, to_questie_map_position};
use indexmap::IndexMap;
use std::collections::HashMap;
use trace_data::stream::emulated_list;
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

const OBJECTIVES: &str = "C_QuestLog.GetQuestObjectives";

/// TS `QuestTriggerEndValue`, JS shape `[text, {[zoneID]: [[x, y], ...]}]`.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestTriggerEndValue {
    pub text: String,
    /// The `{[zoneID]: [[x, y], ...]}` table as (zoneID, coordinates) pairs, in JS property
    /// order with unique zones. Observations and merges hold at most one zone with one pair.
    pub locations: Vec<(f64, Vec<(f64, f64)>)>,
}

/// JS `Math.max` of two numbers: NaN when either is NaN.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

struct CoordTally {
    position: (f64, f64),
    count: usize,
    latest_t: f64,
}

struct ZoneTally {
    id: f64,
    count: usize,
    latest_t: f64,
    /// Keyed like TS's `${x},${y}`: String(n) tells numbers apart exactly as Num does.
    coords: IndexMap<(Num, Num), CoordTally>,
}

/// Zone selection: the preferred zone wins; then more observations, more recent, lower id.
fn better_zone(zone: &ZoneTally, best: &ZoneTally, preferred_zone: Option<f64>) -> bool {
    if let Some(preferred) = preferred_zone {
        let is_preferred = zone.id == preferred;
        if is_preferred != (best.id == preferred) {
            return is_preferred;
        }
    }
    if zone.count != best.count {
        return zone.count > best.count;
    }
    if zone.latest_t != best.latest_t {
        return zone.latest_t > best.latest_t;
    }
    zone.id < best.id
}

/// Merges repeated samples of one quest's completed event objective into Questie's triggerEnd,
/// which carries one zone with one coordinate pair:
///
/// 1. Only samples with the text of the most recent sample count.
/// 2. The zone is `preferred_zone` (the quest's zoneOrSort) when any sample observed it;
///    otherwise the zone with the most observations, then the most recent, then the lowest id.
/// 3. Within that zone the most observed coordinate pair wins; ties go to the most recent.
pub fn merge(observations: &[Observation<QuestTriggerEndValue>], preferred_zone: Option<f64>) -> QuestTriggerEndValue {
    // The first of the latest observations, as `[...observations].sort((a, b) => b.t - a.t)`
    // then `find` read it. A scan instead of the sort, so a NaN time cannot make it panic; JS
    // leaves the order of NaN times to the engine.
    let mut latest: Option<&Observation<QuestTriggerEndValue>> = None;
    for observation in observations.iter().filter(|o| !js::trim(&o.value.text).is_empty()) {
        if latest.is_none_or(|l| observation.t > l.t) {
            latest = Some(observation);
        }
    }
    let Some(latest_text) = latest.map(|o| &o.value.text) else {
        return QuestTriggerEndValue {
            text: String::new(),
            locations: Vec::new(),
        };
    };
    let without_location = || QuestTriggerEndValue {
        text: latest_text.clone(),
        locations: Vec::new(),
    };

    let mut zones: IndexMap<Num, ZoneTally> = IndexMap::new();
    for observation in observations.iter().filter(|o| o.value.text == *latest_text) {
        for (zone_id, positions) in &observation.value.locations {
            // `Number(zoneKey)` of the key `String(zoneID)` gives the number back.
            if !zone_id.is_finite() {
                continue;
            }
            let tally = zones.entry(Num(*zone_id)).or_insert_with(|| ZoneTally {
                id: *zone_id + 0.0,
                count: 0,
                latest_t: 0.0,
                coords: IndexMap::new(),
            });
            tally.count += 1;
            tally.latest_t = js_max(tally.latest_t, observation.t);
            for &position in positions {
                let coord = tally.coords.entry((Num(position.0), Num(position.1))).or_insert(CoordTally {
                    position,
                    count: 0,
                    latest_t: 0.0,
                });
                coord.count += 1;
                coord.latest_t = js_max(coord.latest_t, observation.t);
            }
        }
    }

    let mut best_zone: Option<&ZoneTally> = None;
    for zone in zones.values() {
        if best_zone.is_none_or(|best| better_zone(zone, best, preferred_zone)) {
            best_zone = Some(zone);
        }
    }
    let Some(best_zone) = best_zone else { return without_location() };

    let mut best_coord: Option<&CoordTally> = None;
    for coord in best_zone.coords.values() {
        if best_coord.is_none_or(|best| coord.count > best.count || (coord.count == best.count && coord.latest_t > best.latest_t)) {
            best_coord = Some(coord);
        }
    }
    let Some(best_coord) = best_coord else { return without_location() };

    QuestTriggerEndValue {
        text: latest_text.clone(),
        locations: vec![(best_zone.id, vec![best_coord.position])],
    }
}

pub fn observe(session: &Session) -> Vec<Observation<QuestTriggerEndValue>> {
    let mut observations = Vec::new();
    let Some(objectives_root) = session.function(OBJECTIVES).filter(|root| !root.is_array()) else {
        return observations;
    };

    for quest_id_key in param_keys(objectives_root) {
        let quest_id = js::number(&quest_id_key);
        if !quest_id.is_finite() {
            continue;
        }
        // Objective text -> finished state of its previous positioned sample. Repeated completed
        // samples are stale re-reads, not completions, so only the transition emits.
        let mut previous_finished: HashMap<String, bool> = HashMap::new();

        for entry in get_stream(session, OBJECTIVES, &[KeyRef::Str(&quest_id_key)]).unwrap_or(&[]) {
            let t = entry_t(entry);
            if locale_at(session, t).is_some_and(|locale| locale != "enUS") {
                continue;
            }
            let (Some(zone_id), Some(position)) = (player_zone_at(session, t), player_pos_at(session, t)) else {
                continue;
            };

            let objectives = emulated_list(entry_v(entry)).unwrap_or_default();
            for objective in objectives.into_iter().flatten().filter(|o| o.as_object().is_some()) {
                if objective.get("type").and_then(Value::as_str) != Some("event") {
                    continue;
                }
                let Some(finished) = objective.get("finished") else { continue };
                let Some(text) = objective.get("text").and_then(Value::as_str).map(js::trim).filter(|text| !text.is_empty()) else {
                    continue;
                };

                let finished = finished.as_bool() == Some(true);
                let was_finished = previous_finished.insert(text.to_owned(), finished);
                // Emit on the incomplete->complete transition, or on the first completed sample
                // when the objective was never seen incomplete (e.g. the trace starts after it).
                if !finished || was_finished == Some(true) {
                    continue;
                }

                let questie = to_questie_map_position(position);
                let value = QuestTriggerEndValue {
                    text: text.to_owned(),
                    locations: vec![(zone_id + 0.0, vec![(questie.x, questie.y)])],
                };
                observations.push(Observation::new(quest_id as i64, value, Confidence::High, t));
            }
        }
    }
    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::SessionId;
    use crate::test_support::{session_json, stream, with_session};
    use serde_json::{json, Value as Json};

    fn observed(functions: Json) -> Vec<(i64, QuestTriggerEndValue, Confidence, f64)> {
        with_session(&session_json(functions, vec![]), |s| {
            observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect()
        })
    }

    /// `[text, {zone: [[x, y], ...], ...}]`, zones given in JS property order.
    fn value(text: &str, locations: &[(f64, &[(f64, f64)])]) -> QuestTriggerEndValue {
        QuestTriggerEndValue {
            text: text.to_owned(),
            locations: locations.iter().map(|&(zone, coords)| (zone, coords.to_vec())).collect(),
        }
    }

    fn event_objective(text: &str, finished: bool) -> Json {
        json!([{ "type": "event", "text": text, "finished": finished }])
    }

    #[test]
    fn extracts_a_completed_event_objective_at_the_players_map_position() {
        let functions = json!({
            "C_QuestLog.GetQuestObjectives": { "33": stream([(10.0, event_objective("Light the campfire", true))]) },
            // uiMapID 1436 = Westfall (area 40)
            "C_Map.GetBestMapForUnit": { "player": stream([(10.0, json!(1436))]) },
            "C_Map.GetPlayerMapPosition": { "player": stream([(10.0, json!({ "x": 0.5611, "y": 0.6164 }))]) },
        });
        assert_eq!(
            observed(functions),
            [(33, value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), Confidence::High, 10.0)]
        );
    }

    #[test]
    fn ignores_incomplete_and_non_event_objectives() {
        let functions = json!({
            "C_QuestLog.GetQuestObjectives": { "33": stream([(10.0, json!([
                { "type": "event", "text": "Not completed yet", "finished": false },
                { "type": "monster", "text": "0/1 Kobold Vermin slain", "finished": true },
            ]))]) },
            "C_Map.GetBestMapForUnit": { "player": stream([(10.0, json!(1436))]) },
            "C_Map.GetPlayerMapPosition": { "player": stream([(10.0, json!({ "x": 0.5, "y": 0.5 }))]) },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn skips_non_en_us_objective_text() {
        let functions = json!({
            "GetLocale": { "player": stream([(0.0, json!("deDE"))]) },
            "C_QuestLog.GetQuestObjectives": { "33": stream([(10.0, event_objective("Das Lagerfeuer anzünden", true))]) },
            "C_Map.GetBestMapForUnit": { "player": stream([(10.0, json!(1436))]) },
            "C_Map.GetPlayerMapPosition": { "player": stream([(10.0, json!({ "x": 0.5, "y": 0.5 }))]) },
        });
        assert_eq!(observed(functions), []);
    }

    #[test]
    fn emits_on_the_incomplete_to_complete_transition_not_for_stale_completed_samples() {
        let functions = json!({
            "C_QuestLog.GetQuestObjectives": { "33": stream([
                (5.0, event_objective("Light the campfire", false)),
                (10.0, event_objective("Light the campfire", true)),
                // Stale: the objective stays finished in later re-samples.
                (15.0, event_objective("Light the campfire", true)),
                (20.0, event_objective("Light the campfire", true)),
            ]) },
            "C_Map.GetBestMapForUnit": { "player": stream([(0.0, json!(1436))]) },
            "C_Map.GetPlayerMapPosition": { "player": stream([
                (5.0, json!({ "x": 0.1, "y": 0.2 })),
                (10.0, json!({ "x": 0.5611, "y": 0.6164 })),
                (15.0, json!({ "x": 0.9, "y": 0.9 })),
                (20.0, json!({ "x": 0.8, "y": 0.8 })),
            ]) },
        });
        assert_eq!(
            observed(functions),
            [(33, value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), Confidence::High, 10.0)]
        );
    }

    #[test]
    fn keeps_the_first_completed_sample_when_finished_from_the_first_positioned_sample_on() {
        let functions = json!({
            "C_QuestLog.GetQuestObjectives": { "33": stream([
                (10.0, event_objective("Light the campfire", true)),
                (15.0, event_objective("Light the campfire", true)),
            ]) },
            "C_Map.GetBestMapForUnit": { "player": stream([(0.0, json!(1436))]) },
            "C_Map.GetPlayerMapPosition": { "player": stream([(10.0, json!({ "x": 0.5611, "y": 0.6164 })), (15.0, json!({ "x": 0.9, "y": 0.9 }))]) },
        });
        assert_eq!(
            observed(functions),
            [(33, value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), Confidence::High, 10.0)]
        );
    }

    #[test]
    fn tracks_each_objective_independently_and_skips_samples_without_a_position() {
        let functions = json!({
            "C_QuestLog.GetQuestObjectives": { "33": stream([
                // No position at t=5: the incomplete state is not remembered, so the completed
                // sample at t=10 is a first sighting and emits.
                (5.0, event_objective("Unpositioned", false)),
                (10.0, event_objective("Unpositioned", true)),
                // Another objective reaching completion in the same stream.
                (15.0, event_objective("Light the beacon", false)),
                (20.0, event_objective("Light the beacon", true)),
            ]) },
            "C_Map.GetBestMapForUnit": { "player": stream([(0.0, json!(1436))]) },
            "C_Map.GetPlayerMapPosition": { "player": stream([
                (10.0, json!({ "x": 0.3, "y": 0.4 })),
                (15.0, json!({ "x": 0.5, "y": 0.5 })),
                (20.0, json!({ "x": 0.6, "y": 0.7 })),
            ]) },
        });
        assert_eq!(
            observed(functions),
            [
                (33, value("Unpositioned", &[(40.0, &[(30.0, 40.0)])]), Confidence::High, 10.0),
                (33, value("Light the beacon", &[(40.0, &[(60.0, 70.0)])]), Confidence::High, 20.0),
            ]
        );
    }

    fn obs(value: QuestTriggerEndValue, t: f64) -> Observation<QuestTriggerEndValue> {
        Observation {
            entity_id: 33,
            value,
            confidence: Confidence::High,
            t,
            session: SessionId(0),
        }
    }

    #[test]
    fn merge_exports_exactly_one_zone_with_one_coordinate_pair() {
        let merged = merge(
            &[
                obs(value("Light the campfire", &[(40.0, &[(56.11, 61.64), (57.0, 62.0)])]), 10.0),
                // `{40: ..., 12: ...}` iterates zone 12 first in JS.
                obs(value("Light the campfire", &[(12.0, &[(30.0, 40.0)]), (40.0, &[(56.11, 61.64)])]), 20.0),
            ],
            None,
        );
        assert_eq!(merged, value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]));
    }

    #[test]
    fn merge_prefers_the_quests_zone_or_sort_zone_when_any_sample_observed_it() {
        let merged = merge(
            &[
                obs(value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), 10.0),
                obs(value("Light the campfire", &[(12.0, &[(47.46, 62.18)])]), 20.0),
            ],
            // zoneOrSort resolved Elwynn Forest for this quest
            Some(12.0),
        );
        assert_eq!(merged, value("Light the campfire", &[(12.0, &[(47.46, 62.18)])]));
        // Not in the TS test, where zone 12 also wins by recency: the preferred zone beats a
        // zone with more and newer samples.
        let merged = merge(
            &[
                obs(value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), 10.0),
                obs(value("Light the campfire", &[(12.0, &[(47.46, 62.18)])]), 20.0),
                obs(value("Light the campfire", &[(12.0, &[(47.46, 62.18)])]), 30.0),
            ],
            Some(40.0),
        );
        assert_eq!(merged, value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]));
    }

    #[test]
    fn merge_falls_back_to_the_most_observed_zone_when_the_preferred_one_was_never_observed() {
        let merged = merge(
            &[
                obs(value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), 10.0),
                obs(value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), 20.0),
                obs(value("Light the campfire", &[(12.0, &[(30.0, 40.0)])]), 30.0),
            ],
            Some(999.0),
        );
        assert_eq!(merged, value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]));
    }

    #[test]
    fn merge_breaks_zone_count_ties_by_the_most_recent_sample_then_the_lowest_zone_id() {
        let earlier = obs(value("Light the campfire", &[(12.0, &[(30.0, 40.0)])]), 10.0);
        let later = obs(value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), 20.0);
        // Equal counts: the more recent sample (zone 40) wins.
        assert_eq!(
            merge(&[earlier.clone(), later], None),
            value("Light the campfire", &[(40.0, &[(56.11, 61.64)])])
        );
        // As in the TS test (which only holds zone 12).
        assert_eq!(
            merge(&[earlier.clone(), earlier.clone()], None),
            value("Light the campfire", &[(12.0, &[(30.0, 40.0)])])
        );
        // Equal counts and recency: the lowest zone id wins, whichever zone is met first.
        let same_time = obs(value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), 10.0);
        assert_eq!(merge(&[same_time, earlier], None), value("Light the campfire", &[(12.0, &[(30.0, 40.0)])]));
    }

    #[test]
    fn merge_picks_the_most_observed_coordinate_pair_within_the_winning_zone() {
        let merged = merge(
            &[
                obs(value("Light the campfire", &[(40.0, &[(57.0, 62.0)])]), 10.0),
                obs(value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), 20.0),
                obs(value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]), 30.0),
            ],
            None,
        );
        assert_eq!(merged, value("Light the campfire", &[(40.0, &[(56.11, 61.64)])]));
    }

    #[test]
    fn merge_of_no_samples_has_no_location() {
        assert_eq!(merge(&[], None), value("", &[]));
    }
}
