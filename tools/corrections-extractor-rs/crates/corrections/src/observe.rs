//! Which observers run on a session, and the observations they stage for the fold. Ports
//! `extract/observe.ts` and `extract/observe-lookups.ts`.

use crate::js::format_number;
use crate::observation::Observation;
use crate::observers::quest;
use crate::probes::session_server_time;
use trace_data::{KeyRef, Session, Value};

/// The highest ids Classic knows per entity. Only ids above them are exported, so npc, item
/// and object observations at or below them are dropped right away.
#[derive(Clone, Copy, Debug)]
pub struct MaxIds {
    pub npc: i64,
    pub quest: i64,
    pub item: i64,
    pub object: i64,
}

pub const MAX_IDS: MaxIds = MaxIds {
    npc: 18199,
    quest: 9665,
    item: 25818,
    object: 300_142,
};

/// GetBuildInfo's interface version, as PlayerIdentity.lua records it:
/// `functions.GetBuildInfo.player[0].v`.
fn interface_version(session: &Session) -> Option<f64> {
    let build_info = session.function("GetBuildInfo").filter(|v| v.as_object().is_some())?;
    let first = build_info.get("player")?.as_array()?.first()?;
    first.is_table().then(|| first.get("v")?.as_num()).flatten()
}

/// Sessions from WoW Forever builds (interface 16xxx) are observed. Sessions without
/// GetBuildInfo are included: they may be early Forever traces.
pub fn is_forever_session(session: &Session) -> bool {
    interface_version(session).is_none_or(|version| format_number(version).starts_with("16"))
}

/// The client a session was recorded with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionClient {
    pub interface_version: Option<f64>,
    pub build: Option<f64>,
    pub locale: Option<String>,
}

pub fn session_client(session: &Session) -> SessionClient {
    let build = session
        .field("buildInfo")
        .and_then(|info| info.get("2"))
        .map(number_like)
        .filter(|b| crate::js::is_integer(*b) && *b > 0.0);
    let locale = trace_data::stream::get_stream(session, "GetLocale", &[KeyRef::Str("player")])
        .and_then(|stream| stream.first())
        .and_then(|entry| entry.get("v"))
        .and_then(Value::as_str)
        .filter(|l| !l.is_empty())
        .map(str::to_owned);
    SessionClient {
        interface_version: interface_version(session),
        build,
        locale,
    }
}

/// JS `Number(x)` for a decoded value.
fn number_like(value: &Value) -> f64 {
    match value.resolved() {
        Value::Num(n) => *n,
        Value::Str(s) => crate::js::number(s),
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => f64::NAN,
    }
}

/// One session's observations, ready for the fold.
#[derive(Default)]
pub struct StagedSession {
    pub server_time: Option<f64>,
    pub required_level: Vec<Observation<f64>>,
}

/// Runs every observer over one session; None for sessions that are not observed (not Forever).
pub fn observe_session(session: &Session, max_ids: &MaxIds) -> Option<StagedSession> {
    if !is_forever_session(session) {
        return None;
    }
    let _ = max_ids;
    Some(StagedSession {
        server_time: session_server_time(session),
        required_level: quest::required_level::observe(session),
    })
}
