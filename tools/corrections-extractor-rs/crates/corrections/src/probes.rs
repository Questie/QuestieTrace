//! Domain questions over a session ("what was the player's zone at time t?"), so observers
//! read like the question instead of stream plumbing. Ports `extract/probes.ts`.

use crate::js;
use crate::resources::area_id_for_ui_map_id;
use std::collections::HashSet;
use trace_data::stream::{get_stream, nearest_by_time, unpack, value_at, DEFAULT_CORRELATION_WINDOW_SECONDS};
use trace_data::{KeyRef, Session, Value};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapPosition {
    pub x: f64,
    pub y: f64,
}

/// C_Map's normalized 0-1 coordinates as Questie's 0-100 percentages, two decimals.
pub fn to_questie_map_position(position: MapPosition) -> MapPosition {
    MapPosition { x: js::round(position.x * 10_000.0) / 100.0, y: js::round(position.y * 10_000.0) / 100.0 }
}

/// The value at or before `t` of `name` with `params`, as valueAt(getStream(...) ?? [], t).
pub fn stream_value_at<'v, 'a>(session: &Session<'v, 'a>, name: &str, params: &[KeyRef], t: f64) -> Option<&'v Value<'a>> {
    value_at(get_stream(session, name, params)?, t)
}

fn as_non_empty_str<'v>(value: Option<&'v Value>) -> Option<&'v str> {
    value?.as_str().filter(|s| !s.is_empty())
}

fn as_map_position(value: Option<&Value>) -> Option<MapPosition> {
    let value = value.filter(|v| v.is_table())?;
    Some(MapPosition { x: value.get("x")?.as_num()?, y: value.get("y")?.as_num()? })
}

const PLAYER: [KeyRef<'static>; 1] = [KeyRef::Str("player")];

/// `C_Map.GetBestMapForUnit("player")` at or before `t`, as the AreaTable areaID Questie keys
/// zones by. None when the map has no area (continents, dungeons, unknown ids).
pub fn player_zone_at(session: &Session, t: f64) -> Option<f64> {
    area_id_for_ui_map_id(stream_value_at(session, "C_Map.GetBestMapForUnit", &PLAYER, t).and_then(Value::as_num))
}

/// `C_Map.GetBestMapForUnit("player")` nearest `t` (both directions) within `window`
/// (default: the shared correlation window), as an areaID.
pub fn player_zone_near(session: &Session, t: f64, window: Option<f64>) -> Option<f64> {
    let stream = get_stream(session, "C_Map.GetBestMapForUnit", &PLAYER);
    let window = window.unwrap_or(DEFAULT_CORRELATION_WINDOW_SECONDS);
    area_id_for_ui_map_id(nearest_by_time(stream, t, window).and_then(Value::as_num))
}

/// `C_Map.GetPlayerMapPosition("player")` at or before `t`.
pub fn player_pos_at(session: &Session, t: f64) -> Option<MapPosition> {
    as_map_position(stream_value_at(session, "C_Map.GetPlayerMapPosition", &PLAYER, t))
}

/// `C_Map.GetPlayerMapPosition("player")` nearest `t`, within `window`.
pub fn player_pos_near(session: &Session, t: f64, window: Option<f64>) -> Option<MapPosition> {
    let stream = get_stream(session, "C_Map.GetPlayerMapPosition", &PLAYER);
    as_map_position(nearest_by_time(stream, t, window.unwrap_or(DEFAULT_CORRELATION_WINDOW_SECONDS)))
}

/// `UnitGUID(token)` at or before `t`, e.g. token "target", "npc" or "questnpc".
pub fn unit_guid_at<'v>(session: &Session<'v, '_>, token: &str, t: f64) -> Option<&'v str> {
    as_non_empty_str(stream_value_at(session, "UnitGUID", &[KeyRef::Str(token)], t))
}

/// `UnitName(token)` at or before `t`. UnitName returns packed (name, realm); line breaks some
/// NPC names carry are removed and the result trimmed.
pub fn unit_name_at(session: &Session, token: &str, t: f64) -> Option<String> {
    let raw = stream_value_at(session, "UnitName", &[KeyRef::Str(token)], t)?;
    let name = match unpack(raw) {
        Some(values) => values.first().copied().flatten(),
        None if raw.is_array() => raw.get(0),
        None => Some(raw),
    };
    let name: String = name?.as_str()?.chars().filter(|&c| c != '\r' && c != '\n').collect();
    let name = js::trim(&name);
    (!name.is_empty()).then(|| name.to_owned())
}

/// `GetQuestID()` at or before `t`; 0 ("no active quest frame") is None.
pub fn quest_id_at(session: &Session, t: f64) -> Option<f64> {
    let id = stream_value_at(session, "GetQuestID", &[], t)?.as_num()?;
    // `v && v !== 0`: NaN is falsy too.
    (id != 0.0 && !id.is_nan()).then_some(id)
}

/// `GetTitleText()` at or before `t` (quest detail/gossip frame title).
pub fn quest_title_at<'v>(session: &Session<'v, '_>, t: f64) -> Option<&'v str> {
    as_non_empty_str(stream_value_at(session, "GetTitleText", &[], t))
}

/// `C_QuestLog.GetInfo(questID).title` at or before `t`.
pub fn quest_info_title_at<'v>(session: &Session<'v, '_>, quest_id: f64, t: f64) -> Option<&'v str> {
    let value = stream_value_at(session, "C_QuestLog.GetInfo", &[KeyRef::Num(quest_id)], t)?;
    as_non_empty_str(value.as_object()?.get(KeyRef::Str("title")))
}

/// `GetLocale()` at or before `t`, e.g. "enUS", "deDE".
pub fn locale_at<'v>(session: &Session<'v, '_>, t: f64) -> Option<&'v str> {
    as_non_empty_str(stream_value_at(session, "GetLocale", &PLAYER, t))
}

/// Questie's faction letters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Faction {
    Alliance,
    Horde,
}

impl Faction {
    /// "A" or "H", as Questie writes it.
    pub fn letter(self) -> &'static str {
        match self {
            Faction::Alliance => "A",
            Faction::Horde => "H",
        }
    }
}

/// `UnitFactionGroup("player")` at or before `t`; None when unfactioned or unknown.
pub fn player_faction_at(session: &Session, t: f64) -> Option<Faction> {
    let raw = stream_value_at(session, "UnitFactionGroup", &PLAYER, t)?;
    let faction = if raw.is_table() { raw.get(1)? } else { raw };
    match faction.as_str()? {
        "Alliance" => Some(Faction::Alliance),
        "Horde" => Some(Faction::Horde),
        _ => None,
    }
}

/// The locale-independent race file name of `UnitRace("player")` at or before `t`, e.g. "Human".
pub fn player_race_at<'v>(session: &Session<'v, '_>, t: f64) -> Option<&'v str> {
    let raw = stream_value_at(session, "UnitRace", &PLAYER, t).filter(|v| v.is_table())?;
    as_non_empty_str(raw.get(2))
}

/// The first `GetServerTime()` sample of the session: the realm time around its start.
pub fn session_server_time(session: &Session) -> Option<f64> {
    get_stream(session, "GetServerTime", &[])?.first()?.get("v")?.as_num()
}

/// Completed-quest streams in `functionsDelta`, preferred first (see CompletedQuests.lua).
const COMPLETED_QUEST_STREAMS: [&str; 2] = ["C_QuestLog.GetAllCompletedQuestIDs", "GetQuestsCompleted"];

/// Quest ids, compared as JS numbers (+0 and -0 alike).
#[derive(Default, Debug)]
pub struct QuestSet(HashSet<u64>);

impl QuestSet {
    fn key(id: f64) -> u64 {
        (id + 0.0).to_bits()
    }

    pub fn contains(&self, id: f64) -> bool {
        self.0.contains(&Self::key(id))
    }

    fn insert(&mut self, id: f64) {
        self.0.insert(Self::key(id));
    }

    fn remove(&mut self, id: f64) {
        self.0.remove(&Self::key(id));
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn numbers<'v, 'a>(value: Option<&'v Value<'a>>) -> impl Iterator<Item = f64> + use<'v, 'a> {
    value.and_then(Value::as_array).unwrap_or(&[]).iter().filter_map(Value::as_num)
}

/// probes.ts `completedQuestsAt`: the quests the player had completed at `t` (the stream's
/// initial set with every change at or before `t` applied). None when the session recorded
/// no completed quests, which is not the same as an empty set.
pub fn completed_quests_at(session: &Session, t: f64) -> Option<QuestSet> {
    let streams: Vec<&Value> = COMPLETED_QUEST_STREAMS.iter().filter_map(|name| session.function_delta(name)).collect();
    let has_data = |stream: &&&Value| {
        stream.get("initial").and_then(Value::as_array).is_some_and(|a| !a.is_empty())
            || stream.get("delta").and_then(Value::as_array).is_some_and(|a| !a.is_empty())
    };
    // A client can have the preferred API but leave it empty: a recorded fallback has the data.
    let stream = streams.iter().find(has_data).or(streams.first())?;
    let mut completed = QuestSet::default();
    for id in numbers(stream.get("initial")) {
        completed.insert(id);
    }
    for change in stream.get("delta").and_then(Value::as_array).unwrap_or(&[]) {
        // A change without a time (NaN) never applies, as `!(change.t <= t)` skips it in TS.
        let change_t = change.get("t").and_then(Value::as_num).unwrap_or(f64::NAN);
        let applied_by_t = change_t <= t;
        if !applied_by_t {
            continue;
        }
        for id in numbers(change.get("add")) {
            completed.insert(id);
        }
        for id in numbers(change.get("remove")) {
            completed.remove(id);
        }
    }
    Some(completed)
}
