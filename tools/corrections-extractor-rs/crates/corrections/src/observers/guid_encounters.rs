//! Every point in time a tracked unit token resolved to a GUID of one kind (see
//! Modules/Trackers/UnitInteraction.lua). Port of `extract/observers/_guidEncounters.ts`.
//!
//! NPC and object encounters differ only in the GUID kind they keep: both read the same
//! UnitGUID streams (WoW lets "target" resolve to a GameObject, e.g. a lootable chest).

use crate::guid::{parse_guid, GuidKind};
use trace_data::stream::{entry_t, entry_tp, entry_v, get_stream};
use trace_data::{KeyRef, Value};

/// Unit tokens the addon records UnitGUID/UnitName/UnitLevel for.
pub const TRACKED_UNIT_TOKENS: [&str; 3] = ["target", "npc", "questnpc"];

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GuidEncounter {
    pub entity_id: i64,
    /// The unit token, so field observers can read UnitName(token) etc. at the same `t`.
    pub token: &'static str,
    pub t: f64,
    /// Precise time of the sample, which tells apart samples that share `t`.
    pub tp: f64,
}

/// One encounter per UnitGUID sample of `kind` on `tokens`, token by token. An entity can
/// appear many times; merges deal with the duplicates.
pub fn guid_encounters(session: &trace_data::Session, kind: GuidKind, tokens: &[&'static str]) -> Vec<GuidEncounter> {
    let mut out = Vec::new();
    for &token in tokens {
        let Some(stream) = get_stream(session, "UnitGUID", &[KeyRef::Str(token)]) else {
            continue;
        };
        for entry in stream {
            let Some(guid) = entry_v(entry).and_then(Value::as_str) else { continue };
            let parsed = parse_guid(Some(guid));
            if parsed.kind != kind {
                continue;
            }
            if let Some(id) = parsed.id {
                out.push(GuidEncounter {
                    entity_id: id as i64,
                    token,
                    t: entry_t(entry),
                    tp: entry_tp(entry),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{session_json, with_session};
    use serde_json::json;

    fn encounters(functions: serde_json::Value, kind: GuidKind) -> Vec<(i64, &'static str, f64, f64)> {
        with_session(&session_json(functions, vec![]), |s| {
            guid_encounters(s, kind, &TRACKED_UNIT_TOKENS)
                .into_iter()
                .map(|e| (e.entity_id, e.token, e.t, e.tp))
                .collect()
        })
    }

    #[test]
    fn emits_one_encounter_per_guid_of_the_kind_across_all_tracked_tokens() {
        let functions = json!({ "UnitGUID": {
            "target": [{ "t": 0, "tp": 0, "v": "Creature-0-5208-0-7-823-000031" }],
            "npc": [{ "t": 5, "tp": 5, "v": "Creature-0-5208-0-7-197-000032" }],
            "questnpc": [{ "t": 10, "tp": 10, "v": "Creature-0-5208-0-7-197-000032" }],
        }});
        assert_eq!(
            encounters(functions, GuidKind::Npc),
            [(823, "target", 0.0, 0.0), (197, "npc", 5.0, 5.0), (197, "questnpc", 10.0, 10.0)]
        );
    }

    #[test]
    fn only_matches_the_requested_kind() {
        let functions = json!({ "UnitGUID": { "target": [
            { "t": 0, "tp": 0, "v": "GameObject-0-5208-0-7-2843-0000399" },
            { "t": 1, "tp": 1, "v": "Player-5284-0362" },
            { "t": 2, "tp": 2, "v": "Creature-0-5208-0-7-823-000031" },
        ]}});
        assert_eq!(encounters(functions.clone(), GuidKind::Object), [(2843, "target", 0.0, 0.0)]);
        assert_eq!(encounters(functions, GuidKind::Npc), [(823, "target", 2.0, 2.0)]);
    }

    #[test]
    fn returns_nothing_without_a_unit_guid_stream() {
        assert_eq!(encounters(json!({}), GuidKind::Npc), []);
    }
}
