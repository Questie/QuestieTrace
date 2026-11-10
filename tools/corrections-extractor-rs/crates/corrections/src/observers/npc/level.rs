//! npcKeys.minLevel / npcKeys.maxLevel - Questie fields 4 and 5. Port of
//! `extract/observers/npc/level.ts`.
//!
//! UnitLevel[token] from Modules/Trackers/UnitState.lua, paired with the NPC's GUID (see
//! unit_state.rs). Levels below 1 (-1 is "??", e.g. skull bosses) are skipped.
//!
//! Min and max react to a single wrong level, so a level only counts when at least
//! MIN_SESSIONS sessions saw it and it was seen in at least MIN_SHARE of the sessions of the
//! NPC's most common level. That drops rare wrong pairings (e.g. 22 next to 200 sessions of
//! 23-24) but keeps real ranges of spawns with mixed levels.

use crate::aggregate::Num;
use crate::js;
use crate::observation::{Confidence, Observation, SessionId};
use crate::observers::npc::unit_state::unit_state_encounters;
use indexmap::{IndexMap, IndexSet};
use std::collections::HashSet;
use trace_data::Session;

/// Written out as the separate minLevel and maxLevel fields.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NpcLevelRange {
    pub min: f64,
    pub max: f64,
}

const MIN_SESSIONS: usize = 2;
const MIN_SHARE: f64 = 0.1;

/// Each level of each NPC once per session.
pub fn observe(session: &Session) -> Vec<Observation<f64>> {
    let mut observations = Vec::new();
    let mut seen: HashSet<(i64, Num)> = HashSet::new();
    for encounter in unit_state_encounters(session, "UnitLevel", &[]) {
        // `typeof level !== "number" || level < 1`: NaN is a number and not below 1.
        let Some(level) = encounter.value.as_num().filter(|&level| level >= 1.0 || level.is_nan()) else {
            continue;
        };
        if !seen.insert((encounter.npc_id, Num(level))) {
            continue;
        }
        observations.push(Observation::new(encounter.npc_id, level, Confidence::High, encounter.t));
    }
    observations
}

/// None when no level was seen often enough.
pub fn merge(observations: &[Observation<f64>]) -> Option<NpcLevelRange> {
    let mut sessions_by_level: IndexMap<Num, IndexSet<SessionId>> = IndexMap::new();
    for obs in observations {
        // A JS Map stores a -0 key as +0.
        sessions_by_level.entry(Num(obs.value + 0.0)).or_default().insert(obs.session);
    }
    let top = sessions_by_level.values().map(IndexSet::len).max().unwrap_or(0);
    let levels: Vec<f64> = sessions_by_level
        .iter()
        .filter(|(_, sessions)| sessions.len() >= MIN_SESSIONS && sessions.len() as f64 >= top as f64 * MIN_SHARE)
        .map(|(level, _)| level.0)
        .collect();
    if levels.is_empty() {
        return None;
    }
    Some(NpcLevelRange {
        min: js::min(&levels),
        max: js::max(&levels),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{event, packed, session_json, with_session};
    use serde_json::{json, Value as Json};

    fn creature_guid(id: i64) -> String {
        format!("Creature-0-5208-0-7-{id}-000032")
    }

    /// TS `apiSample`: a stream entry; a null value is recorded as nil.
    fn sample(t: f64, tp: f64, v: Json) -> Json {
        if v.is_null() {
            json!({ "t": t, "tp": tp })
        } else {
            json!({ "t": t, "tp": tp, "v": v })
        }
    }

    fn trace_event(name: &str, t: f64) -> Json {
        event(t, name, packed(&[]))
    }

    fn observed(functions: Json, events: Vec<Json>) -> Vec<(i64, f64, Confidence, f64)> {
        with_session(&session_json(functions, events), |s| {
            observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect()
        })
    }

    fn sighting(session: u32, level: f64) -> Observation<f64> {
        Observation {
            session: SessionId(session),
            ..Observation::new(1, level, Confidence::High, 0.0)
        }
    }

    #[test]
    fn records_the_level_sampled_with_the_target_guid() {
        let functions = json!({
            "UnitGUID": { "target": [sample(5.0, 5.0, creature_guid(200001).into())] },
            "UnitLevel": { "target": [sample(5.0, 5.0, 12.into())] },
        });
        assert_eq!(observed(functions, vec![]), [(200001, 12.0, Confidence::High, 5.0)]);
    }

    #[test]
    fn uses_an_unchanged_level_when_the_mouseover_event_fired_at_the_guid_sample() {
        let functions = json!({
            "UnitGUID": { "mouseover": [sample(3.0, 3.0, creature_guid(200001).into()), sample(8.0, 8.0, creature_guid(200002).into())] },
            "UnitLevel": { "mouseover": [sample(3.0, 3.0, 12.into())] },
        });
        let events = vec![trace_event("UPDATE_MOUSEOVER_UNIT", 3.0), trace_event("UPDATE_MOUSEOVER_UNIT", 8.0)];
        let levels: Vec<(i64, f64)> = observed(functions, events).into_iter().map(|(id, level, ..)| (id, level)).collect();
        assert_eq!(levels, [(200001, 12.0), (200002, 12.0)]);
    }

    #[test]
    fn does_not_pair_a_stale_level_with_a_guid_sampled_at_another_event() {
        let functions = json!({
            "UnitGUID": { "target": [sample(3.0, 3.0, creature_guid(200001).into()), sample(8.0, 8.0, creature_guid(200002).into())] },
            "UnitLevel": { "target": [sample(3.0, 3.0, 12.into())] },
        });
        let events = vec![trace_event("PLAYER_TARGET_CHANGED", 3.0), trace_event("QUEST_DETAIL", 8.0)];
        let ids: Vec<i64> = observed(functions, events).into_iter().map(|(id, ..)| id).collect();
        assert_eq!(ids, [200001]);
    }

    #[test]
    fn pairs_each_guid_with_its_own_level_when_the_target_changed_twice_at_the_same_time() {
        let functions = json!({
            "UnitGUID": { "target": [sample(5.0, 5.1, creature_guid(200001).into()), sample(5.0, 5.3, creature_guid(200002).into())] },
            "UnitLevel": { "target": [sample(5.0, 5.2, 12.into()), sample(5.0, 5.4, 30.into())] },
        });
        let levels: Vec<(i64, f64)> = observed(functions, vec![]).into_iter().map(|(id, level, ..)| (id, level)).collect();
        assert_eq!(levels, [(200001, 12.0), (200002, 30.0)]);
    }

    #[test]
    fn skips_unknown_levels() {
        let functions = json!({
            "UnitGUID": { "target": [sample(5.0, 5.0, creature_guid(200001).into())] },
            "UnitLevel": { "target": [sample(5.0, 5.0, (-1).into())] },
        });
        assert_eq!(observed(functions, vec![]), []);
    }

    #[test]
    fn records_each_level_of_an_npc_once_per_session() {
        let functions = json!({
            "UnitGUID": { "target": [sample(3.0, 3.0, creature_guid(200001).into()), sample(4.0, 4.0, Json::Null), sample(5.0, 5.0, creature_guid(200001).into())] },
            "UnitLevel": { "target": [sample(3.0, 3.0, 12.into()), sample(4.0, 4.0, 30.into()), sample(5.0, 5.0, 12.into())] },
        });
        let levels: Vec<(i64, f64)> = observed(functions, vec![]).into_iter().map(|(id, level, ..)| (id, level)).collect();
        assert_eq!(levels, [(200001, 12.0)]);
    }

    #[test]
    fn merge_returns_the_lowest_and_highest_level() {
        let range = merge(&[sighting(1, 12.0), sighting(2, 12.0), sighting(3, 13.0), sighting(4, 13.0)]);
        assert_eq!(range, Some(NpcLevelRange { min: 12.0, max: 13.0 }));
    }

    #[test]
    fn merge_ignores_levels_seen_in_only_one_session() {
        assert_eq!(
            merge(&[sighting(1, 12.0), sighting(2, 12.0), sighting(3, 20.0)]),
            Some(NpcLevelRange { min: 12.0, max: 12.0 })
        );
    }

    #[test]
    fn merge_ignores_levels_seen_far_less_often_than_the_most_common_level() {
        let mut observations: Vec<Observation<f64>> = (0..30).map(|session| sighting(session, 23.0)).collect();
        observations.extend([sighting(100, 22.0), sighting(101, 22.0)]);
        assert_eq!(merge(&observations), Some(NpcLevelRange { min: 23.0, max: 23.0 }));
    }

    #[test]
    fn merge_counts_one_session_once_per_level() {
        assert_eq!(merge(&[sighting(1, 12.0), sighting(1, 12.0)]), None);
    }
}
