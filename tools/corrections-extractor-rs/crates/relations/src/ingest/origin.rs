//! Where a session was recorded. Only the live game counts.
//!
//! The beta (until October 23, 2026) had other quest data, and so does a server that is not
//! Blizzard's. One such session is enough to hide a relation: a prerequisite a single character
//! lacked is no prerequisite to the signals. On the archive of November 23, 2026 they were the
//! only contradictions of seven hand-written prerequisites ("Rascally Rodents" needs "Book
//! Inventory"), see RELATIONS.md, "Beta and other servers".

use trace_data::stream::{entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

/// Epoch seconds of 2026-11-04 00:00 UTC. The last beta session in the archive is from October 23,
/// the first one on the live realms from November 4, 23:05 UTC.
pub const LIVE_SINCE: f64 = 1_793_750_400.0;

/// Blizzard's servers had ids from 4389 to 7122 in the archive of November 23, 2026. The six
/// characters recorded on another server had 1 and 70, as private servers number their realms.
const LOWEST_BLIZZARD_SERVER: u32 = 1000;

/// Whether a server id (see [`server_of`]) is one of Blizzard's.
pub fn is_blizzard(server_id: u32) -> bool {
    server_id >= LOWEST_BLIZZARD_SERVER
}

/// The game a session was recorded in, by the server's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Era {
    Beta,
    Live,
    /// No server time was recorded.
    Undated,
}

/// The era of a session that started at `start_server_time` (GetServerTime, epoch seconds).
pub fn era(start_server_time: Option<f64>) -> Era {
    match start_server_time {
        None => Era::Undated,
        Some(start) if start < LIVE_SINCE => Era::Beta,
        Some(_) => Era::Live,
    }
}

/// The id of the session's server: the one in the first NPC or object GUID it recorded. None for a
/// session that met no NPC.
pub fn server_of(session: &Session) -> Option<u32> {
    let guids = session.function("UnitGUID")?;
    param_keys(guids).iter().find_map(|token| {
        let stream = get_stream(session, "UnitGUID", &[KeyRef::Str(token)])?;
        stream.iter().filter_map(entry_v).filter_map(Value::as_str).find_map(server_id)
    })
}

/// The server id in an NPC's or object's GUID, "Creature-0-<server>-<map>-<zone>-<id>-<spawn>".
fn server_id(guid: &str) -> Option<u32> {
    let mut parts = guid.split('-');
    if !matches!(parts.next()?, "Creature" | "Vehicle" | "GameObject") {
        return None;
    }
    parts.nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::test_trace::TraceBuilder;
    use serde_json::json;

    #[test]
    fn a_session_is_beta_until_the_live_realms_opened() {
        // node: Date.UTC(2026, 9, 23, 15, 32) / 1000 (the last beta session) and
        // Date.UTC(2026, 10, 4, 23, 5) / 1000 (the first live one).
        assert_eq!(era(Some(1_792_769_520.0)), Era::Beta);
        assert_eq!(era(Some(1_793_833_500.0)), Era::Live);
        assert_eq!(era(None), Era::Undated);
    }

    #[test]
    fn the_server_is_told_by_the_id_in_its_npc_guids() {
        let met = |guid: &str| TraceBuilder::new().sample_param("UnitGUID", "target", json!(guid)).build().server_id;
        assert_eq!(met("Creature-0-70-0-0-951-B1D7514EC9"), Some(70));
        assert_eq!(met("GameObject-0-1-0-12-3972-0000AB"), Some(1));
        assert_eq!(met("Creature-0-6823-0-12-951-00002D41D1"), Some(6823));
        // Pets, items and broken GUIDs say nothing about the server.
        assert_eq!(met("Pet-0-70-0-0-951-B1D7514EC9"), None);
        assert_eq!(met("Item-70-0-400000"), None);
        assert_eq!(met("Creature-0"), None);
        assert_eq!(TraceBuilder::new().build().server_id, None, "a session that met no NPC");
        // The smallest and largest id on Blizzard's servers, and the two on others.
        assert!(is_blizzard(4389) && is_blizzard(7122));
        assert!(!is_blizzard(1) && !is_blizzard(70));
    }

    #[test]
    fn a_session_with_no_npc_under_one_token_is_told_by_another() {
        let session = TraceBuilder::new().sample_param("UnitGUID", "npc", serde_json::Value::Null).sample_param(
            "UnitGUID",
            "target",
            json!("Creature-0-70-0-0-951-B1D7514EC9"),
        );
        assert_eq!(session.build().server_id, Some(70));
    }
}
