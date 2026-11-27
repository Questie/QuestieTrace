//! Where a session was recorded. Only the live game counts.
//!
//! The beta (until October 23, 2026) had other quest data. One beta session is enough to hide a
//! relation: a prerequisite a single character lacked is no prerequisite to the signals. The
//! server id in NPC GUIDs cannot tell another game apart: Blizzard's layers and regions use ids
//! from 0 up. See RELATIONS.md, "The beta".

/// Epoch seconds of 2026-11-04 00:00 UTC. The last beta session in the archive is from October 23,
/// the first one on the live realms from November 4, 23:05 UTC.
pub const LIVE_SINCE: f64 = 1_793_750_400.0;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_beta_until_the_live_realms_opened() {
        // node: Date.UTC(2026, 9, 23, 15, 32) / 1000 (the last beta session) and
        // Date.UTC(2026, 10, 4, 23, 5) / 1000 (the first live one).
        assert_eq!(era(Some(1_792_769_520.0)), Era::Beta);
        assert_eq!(era(Some(1_793_833_500.0)), Era::Live);
        assert_eq!(era(None), Era::Undated);
    }
}
