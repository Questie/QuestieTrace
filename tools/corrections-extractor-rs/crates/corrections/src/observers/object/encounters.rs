//! When and through which unit token a session saw each objectID: the anchors object field
//! observers read other streams at. Port of `extract/observers/object/_encounters.ts`.

use crate::guid::GuidKind;
use crate::observers::guid_encounters::{guid_encounters, TRACKED_UNIT_TOKENS};
use trace_data::Session;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ObjectEncounter {
    pub object_id: i64,
    /// The unit token, so field observers can read UnitName(token) etc. at the same `t`.
    pub token: &'static str,
    pub t: f64,
}

pub fn object_encounters(session: &Session) -> Vec<ObjectEncounter> {
    guid_encounters(session, GuidKind::Object, &TRACKED_UNIT_TOKENS)
        .into_iter()
        .map(|e| ObjectEncounter { object_id: e.entity_id, token: e.token, t: e.t })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::json;

    fn encounters(functions: serde_json::Value) -> Vec<(i64, &'static str, f64)> {
        with_session(&session_json(functions, vec![]), |s| object_encounters(s).into_iter().map(|e| (e.object_id, e.token, e.t)).collect())
    }

    #[test]
    fn emits_one_encounter_per_game_object_guid_across_all_tracked_tokens() {
        let functions = json!({ "UnitGUID": { "target": [{ "t": 0, "tp": 0, "v": "GameObject-0-5208-0-7-2843-0000399" }] } });
        assert_eq!(encounters(functions), [(2843, "target", 0.0)]);
    }

    #[test]
    fn ignores_creature_and_player_guids_on_the_same_tokens() {
        let functions = json!({ "UnitGUID": { "target": [
            { "t": 0, "tp": 0, "v": "Creature-0-5208-0-7-823-000031" },
            { "t": 1, "tp": 1, "v": "Player-5284-0362" },
        ]}});
        assert_eq!(encounters(functions), []);
    }

    #[test]
    fn returns_nothing_without_a_unit_guid_stream() {
        assert_eq!(encounters(json!({})), []);
    }
}
