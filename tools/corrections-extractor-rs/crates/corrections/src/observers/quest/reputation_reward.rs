//! questKeys.reputationReward - Questie field 26: `{{faction, value}, ...}`, the reputation a
//! quest rewards. Port of `extract/observers/quest/reputationReward.ts`.
//!
//! Source: the change of each faction's reputation around QUEST_TURNED_IN, read from
//! C_Reputation.GetFactionDataByID (currentStanding) or, for a faction without such a stream,
//! GetFactionInfoByID (barValue). The client updates reputation shortly before or after the
//! turn-in event, so a gain is the value at turn-in + CHANGE_WINDOW minus the last value
//! strictly before turn-in - CHANGE_WINDOW. A turn-in is skipped when another one happened
//! within TURN_IN_ISOLATION seconds. This works in every client language.
//!
//! A turn-in can unlock a faction, which is then first seen inside the window, or only shows up
//! in the reputation streams minutes later. The second case is caught by counting reputation
//! messages with an amount (one per changed faction, in every language): more messages than
//! changes means a faction is missing. Such turn-ins fall back to the enUS "Reputation with %s
//! increased by %d." messages (CHAT_MSG_COMBAT_FACTION_CHANGE) within CHANGE_WINDOW, the faction
//! name mapped to its id by the session's own reputation streams; other locales skip them.
//!
//! Gains are changed by things the trace cannot see in full, so the merge is strict:
//! - Human sessions, and sessions without a known race, are skipped (Diplomacy adds 10%).
//! - Spillover (25% of another faction's gain, printed rounded either way) is not merged by
//!   amount. A spillover faction is kept when it passes the presence and session checks below
//!   and spills over from the same decided reward in at least MIN_AMOUNT_SHARE of its turn-ins;
//!   its amount is that reward / 4, unrounded.
//! - A faction is kept when it shows up in at least half of the quest's turn-ins with
//!   reputation, in at least MIN_SESSIONS sessions, and one amount covers at least
//!   MIN_AMOUNT_SHARE of those sessions.
//! - Turn-ins without any change are ignored: every faction was capped.
//! - A quest is dropped when both Alliance and Horde turned it in and their results differ,
//!   including when one side has too little data to decide.

use crate::aggregate::Num;
use crate::js;
use crate::observation::{Confidence, Observation, SessionId};
use crate::probes::{locale_at, player_faction_at, player_race_at, Faction};
use indexmap::{IndexMap, IndexSet};
use std::collections::{HashMap, HashSet};
use trace_data::stream::{entry_t, entry_v, get_stream, param_keys};
use trace_data::{KeyRef, Session, Value};

/// The merged reward: (factionID, amount) pairs sorted by faction. JS shape
/// `[[factionID, amount], ...]`.
pub type QuestReputationRewardValue = Vec<(f64, f64)>;

/// A JS object keyed by faction id (`Record<number, number>`): its entries in JS property
/// order, array-index ids ascending, then other ids in insertion order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FactionRecord(Vec<(f64, f64)>);

impl crate::memory::HeapSize for FactionRecord {
    fn heap_size(&self) -> usize {
        self.0.capacity() * std::mem::size_of::<(f64, f64)>()
    }
}

impl FactionRecord {
    /// Whether `String(id)` is an array index, which JS orders before other keys.
    fn is_array_index(id: f64) -> bool {
        id.fract() == 0.0 && (0.0..=4_294_967_294.0).contains(&id)
    }

    /// `record[id] = value`: an existing id keeps its position.
    fn insert(&mut self, id: f64, value: f64) {
        // String(-0) is "0".
        let id = id + 0.0;
        if let Some(entry) = self.0.iter_mut().find(|(key, _)| *key == id) {
            entry.1 = value;
            return;
        }
        let at = if Self::is_array_index(id) {
            self.0
                .iter()
                .position(|&(key, _)| !Self::is_array_index(key) || key > id)
                .unwrap_or(self.0.len())
        } else {
            self.0.len()
        };
        self.0.insert(at, (id, value));
    }

    fn contains(&self, id: f64) -> bool {
        self.0.iter().any(|&(key, _)| key == id)
    }

    /// `Object.entries`, ids as numbers.
    pub fn iter(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.0.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Tests write records as literals; entries are inserted in the order given.
#[cfg(test)]
impl<const N: usize> From<[(f64, f64); N]> for FactionRecord {
    fn from(entries: [(f64, f64); N]) -> Self {
        let mut record = FactionRecord::default();
        for (id, value) in entries {
            record.insert(id, value);
        }
        record
    }
}

/// One isolated turn-in's reputation changes. JS shape `{playerFaction, gains, spillover}`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReputationTurnInValue {
    pub player_faction: Faction,
    /// factionID -> reputation change, without spillover.
    pub gains: FactionRecord,
    /// factionID -> factionID of the gain it spilled over from.
    pub spillover: FactionRecord,
}

const CHANGE_WINDOW: f64 = 0.5;
const TURN_IN_ISOLATION: f64 = 1.0;
const MIN_SESSIONS: usize = 2;
const MIN_AMOUNT_SHARE: f64 = 0.8;
const MIN_PRESENCE_SHARE: f64 = 0.5;
const SPILLOVER_RATIO: f64 = 4.0;
const SPILLOVER_TOLERANCE: f64 = 1.0;

#[derive(Clone, Copy, PartialEq)]
enum FactionApi {
    /// C_Reputation.GetFactionDataByID, preferred.
    FactionData,
    /// GetFactionInfoByID.
    FactionInfo,
}

impl FactionApi {
    const ALL: [FactionApi; 2] = [FactionApi::FactionData, FactionApi::FactionInfo];

    fn function_name(self) -> &'static str {
        match self {
            FactionApi::FactionData => "C_Reputation.GetFactionDataByID",
            FactionApi::FactionInfo => "GetFactionInfoByID",
        }
    }

    /// The reputation value of one sample: currentStanding, or the legacy barValue.
    fn standing(self, value: &Value) -> Option<f64> {
        match self {
            FactionApi::FactionData => value.get("currentStanding")?.as_num(),
            FactionApi::FactionInfo => value.get(6)?.as_num(),
        }
    }

    fn name<'v>(self, value: &'v Value) -> Option<&'v str> {
        match self {
            FactionApi::FactionData => value.get("name")?.as_str(),
            FactionApi::FactionInfo => value.get(1)?.as_str(),
        }
    }
}

struct StandingSample {
    t: f64,
    /// None when the sample holds no standing.
    standing: Option<f64>,
}

/// The faction streams of each api with a numeric key, as (factionID, key, entries).
fn faction_streams<'v, 'a>(session: &Session<'v, 'a>, api: FactionApi) -> Vec<(f64, &'v [Value<'a>])> {
    let Some(root) = session.function(api.function_name()).filter(|root| !root.is_array()) else {
        return Vec::new();
    };
    param_keys(root)
        .iter()
        .filter_map(|key| {
            let faction_id = js::number(key);
            js::is_integer(faction_id).then(|| (faction_id, get_stream(session, api.function_name(), &[KeyRef::Str(key)]).unwrap_or(&[])))
        })
        .collect()
}

/// factionID -> standing streams, C_Reputation before the legacy api.
fn standing_streams(session: &Session) -> IndexMap<Num, Vec<Vec<StandingSample>>> {
    let mut streams: IndexMap<Num, Vec<Vec<StandingSample>>> = IndexMap::new();
    for api in FactionApi::ALL {
        for (faction_id, entries) in faction_streams(session, api) {
            if entries.is_empty() {
                continue;
            }
            // `!value || typeof value !== "object"`: only tables have a standing.
            let samples = entries.iter().map(|entry| StandingSample {
                t: entry_t(entry),
                standing: entry_v(entry).filter(|v| v.is_table()).and_then(|v| api.standing(v)),
            });
            streams.entry(Num(faction_id)).or_default().push(samples.collect());
        }
    }
    streams
}

fn faction_ids_by_name(session: &Session) -> HashMap<String, f64> {
    let mut ids = HashMap::new();
    for api in FactionApi::ALL {
        for (faction_id, entries) in faction_streams(session, api) {
            for value in entries.iter().filter_map(entry_v).filter(|v| v.is_table()) {
                if let Some(name) = api.name(value).filter(|name| !name.is_empty()) {
                    ids.insert(name.to_owned(), faction_id);
                }
            }
        }
    }
    ids
}

/// `/^Reputation with (.+) (increased|decreased) by (\d+)\.$/` as (faction name, signed amount).
fn parse_reputation_message(text: &str) -> Option<(&str, f64)> {
    // `.` matches no line terminator, and the rest of the pattern holds none.
    if text.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    let rest = text.strip_prefix("Reputation with ")?.strip_suffix('.')?;
    // The digits run to the final '.', so they are the whole digit run before it.
    let digit_count = rest.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 {
        return None;
    }
    let (rest, digits) = rest.split_at(rest.len() - digit_count);
    let rest = rest.strip_suffix(" by ")?;
    let (name, sign) = match (rest.strip_suffix(" increased"), rest.strip_suffix(" decreased")) {
        (Some(name), _) => (name, 1.0),
        (None, Some(name)) => (name, -1.0),
        (None, None) => return None,
    };
    (!name.is_empty()).then(|| (name, sign * js::number(digits)))
}

/// Gains from the enUS chat `messages` of a turn-in; None when a message cannot be used.
fn chat_gains(messages: &[String], faction_ids: &HashMap<String, f64>) -> Option<FactionRecord> {
    let mut gains = FactionRecord::default();
    for text in messages {
        let Some((name, amount)) = parse_reputation_message(text) else { continue };
        let faction_id = *faction_ids.get(name)?;
        if gains.contains(faction_id) {
            return None;
        }
        gains.insert(faction_id, amount);
    }
    Some(gains)
}

/// Last standing at or before `t` (strictly before when `strict`): None when nothing was
/// sampled yet (TS undefined), Some(None) for a sample without standing (TS null).
fn standing_at(stream: &[StandingSample], t: f64, strict: bool) -> Option<Option<f64>> {
    let mut result = None;
    for sample in stream {
        if if strict { sample.t >= t } else { sample.t > t } {
            break;
        }
        result = Some(sample.standing);
    }
    result
}

/// Gains from the reputation values around `t`; None when a faction was first seen within
/// CHANGE_WINDOW.
fn standing_gains(streams: &IndexMap<Num, Vec<Vec<StandingSample>>>, t: f64) -> Option<FactionRecord> {
    let mut gains = FactionRecord::default();
    for (faction_id, faction_streams) in streams {
        let boundaries: Vec<_> = faction_streams
            .iter()
            .map(|stream| (standing_at(stream, t - CHANGE_WINDOW, true), standing_at(stream, t + CHANGE_WINDOW, false)))
            .collect();
        // The first stream with a standing at both boundaries wins; otherwise the preferred one.
        let (before, after) = boundaries
            .iter()
            .find(|(before, after)| matches!((before, after), (Some(Some(_)), Some(Some(_)))))
            .unwrap_or(&boundaries[0]);
        let Some(after) = after else { continue };
        let Some(before) = before else { return None };
        let (Some(before), Some(after)) = (before, after) else { continue };
        if before == after {
            continue;
        }
        gains.insert(faction_id.0, after - before);
    }
    Some(gains)
}

/// Splits off gains that spill over from a larger gain in the same direction of the same
/// turn-in; the largest matching gain is the source. Returns (gains, spillover).
fn split_spillover(all_gains: &FactionRecord) -> (FactionRecord, FactionRecord) {
    let mut gains = FactionRecord::default();
    let mut spillover = FactionRecord::default();
    for (faction_id, amount) in all_gains.iter() {
        let is_source =
            |other: f64| js::sign(other) == js::sign(amount) && other.abs() > amount.abs() && (amount - other / SPILLOVER_RATIO).abs() <= SPILLOVER_TOLERANCE;
        // The first of the largest, as a stable sort by |amount| descending would put first.
        let mut source: Option<(f64, f64)> = None;
        for (other_id, other) in all_gains.iter().filter(|&(_, other)| is_source(other)) {
            if source.is_none_or(|(_, best)| other.abs() > best.abs()) {
                source = Some((other_id, other));
            }
        }
        match source {
            Some((source_id, _)) => spillover.insert(faction_id, source_id),
            None => gains.insert(faction_id, amount),
        }
    }
    (gains, spillover)
}

/// QUEST_TURNED_IN's questID (argument 1), when positive.
fn turn_in_quest_id(args: Option<&Value>) -> Option<f64> {
    args?.get(1)?.as_num().filter(|&id| id > 0.0)
}

/// JS `String(value)` of a chat message argument.
fn message_text(value: Option<&Value>) -> String {
    crate::js::string(value)
}

pub fn observe(session: &Session) -> Vec<Observation<ReputationTurnInValue>> {
    let mut observations = Vec::new();
    let event_count = session.event_count();
    let event = |i: usize| session.event(i).expect("index below event_count");
    let mut streams = None;
    let mut faction_ids = None;

    for i in 0..event_count {
        let turn_in = event(i);
        if !turn_in.is("QUEST_TURNED_IN") {
            continue;
        }
        let Some(quest_id) = turn_in_quest_id(turn_in.args()) else { continue };
        let t = turn_in.t();
        if matches!(player_race_at(session, t), None | Some("Human")) {
            continue;
        }
        let Some(player_faction) = player_faction_at(session, t) else { continue };

        let mut start = i;
        while start > 0 && event(start - 1).t() > t - TURN_IN_ISOLATION {
            start -= 1;
        }
        let mut messages = Vec::new();
        let mut isolated = true;
        for j in (start..event_count).take_while(|&j| event(j).t() < t + TURN_IN_ISOLATION) {
            let other = event(j);
            if j != i && other.is("QUEST_TURNED_IN") {
                isolated = false;
                break;
            }
            // TS reads `event.a[1]`; an event without arguments would throw there.
            if other.is("CHAT_MSG_COMBAT_FACTION_CHANGE") && (other.t() - t).abs() <= CHANGE_WINDOW {
                messages.push(message_text(other.arg(1)));
            }
        }
        if !isolated {
            continue;
        }

        let streams = streams.get_or_insert_with(|| standing_streams(session));
        let mut gains = standing_gains(streams, t);
        let amount_messages = messages.iter().filter(|text| text.bytes().any(|b| b.is_ascii_digit())).count();
        if gains.as_ref().is_none_or(|gains| amount_messages > gains.len()) {
            if locale_at(session, t) != Some("enUS") {
                continue;
            }
            let faction_ids = faction_ids.get_or_insert_with(|| faction_ids_by_name(session));
            gains = chat_gains(&messages, faction_ids);
        }
        let Some(gains) = gains.filter(|gains| !gains.is_empty()) else { continue };

        let (gains, spillover) = split_spillover(&gains);
        let value = ReputationTurnInValue {
            player_faction,
            gains,
            spillover,
        };
        observations.push(Observation::new(quest_id as i64, value, Confidence::High, t));
    }
    observations
}

/// The first entry with the largest `size`, as a stable descending sort puts first.
fn first_largest<K, V>(entries: &IndexMap<K, V>, size: impl Fn(&V) -> usize) -> Option<(&K, &V)> {
    let mut best: Option<(&K, &V)> = None;
    for (key, value) in entries {
        if best.is_none_or(|(_, best)| size(value) > size(best)) {
            best = Some((key, value));
        }
    }
    best
}

fn merge_for_player_faction(observations: &[&Observation<ReputationTurnInValue>]) -> Option<QuestReputationRewardValue> {
    if observations.is_empty() {
        return None;
    }
    let presence_needed = observations.len() as f64 * MIN_PRESENCE_SHARE;

    // factionID -> amount -> sessions; JS Map keys turn -0 into 0.
    let mut sessions_by_faction: IndexMap<Num, IndexMap<Num, IndexSet<SessionId>>> = IndexMap::new();
    let mut turn_ins_by_faction: HashMap<Num, usize> = HashMap::new();
    for obs in observations {
        for (faction_id, amount) in obs.value.gains.iter() {
            *turn_ins_by_faction.entry(Num(faction_id)).or_default() += 1;
            sessions_by_faction
                .entry(Num(faction_id))
                .or_default()
                .entry(Num(amount + 0.0))
                .or_default()
                .insert(obs.session);
        }
    }

    let mut rewards: QuestReputationRewardValue = Vec::new();
    for (faction_id, amounts) in &sessions_by_faction {
        if (turn_ins_by_faction[faction_id] as f64) < presence_needed {
            continue;
        }
        let all_sessions: HashSet<SessionId> = amounts.values().flatten().copied().collect();
        if all_sessions.len() < MIN_SESSIONS {
            continue;
        }
        let Some((amount, sessions)) = first_largest(amounts, IndexSet::len) else {
            continue;
        };
        if sessions.len() < MIN_SESSIONS || (sessions.len() as f64) < all_sessions.len() as f64 * MIN_AMOUNT_SHARE {
            continue;
        }
        rewards.push((faction_id.0, amount.0));
    }

    let decided: HashMap<Num, f64> = rewards.iter().map(|&(faction_id, amount)| (Num(faction_id), amount)).collect();
    // factionID -> source factionID -> turn-ins.
    let mut spillover_sources: IndexMap<Num, IndexMap<Num, usize>> = IndexMap::new();
    let mut spillover_sessions: HashMap<Num, HashSet<SessionId>> = HashMap::new();
    for obs in observations {
        for (faction_id, source) in obs.value.spillover.iter() {
            *spillover_sources.entry(Num(faction_id)).or_default().entry(Num(source + 0.0)).or_default() += 1;
            spillover_sessions.entry(Num(faction_id)).or_default().insert(obs.session);
        }
    }
    for (faction_id, sources) in &spillover_sources {
        if decided.contains_key(faction_id) {
            continue;
        }
        let turn_ins: usize = sources.values().sum();
        if (turn_ins as f64) < presence_needed {
            continue;
        }
        if spillover_sessions.get(faction_id).map_or(0, HashSet::len) < MIN_SESSIONS {
            continue;
        }
        let Some((source, &count)) = first_largest(sources, |&count| count) else {
            continue;
        };
        let Some(source_amount) = decided.get(source) else { continue };
        if (count as f64) < turn_ins as f64 * MIN_AMOUNT_SHARE {
            continue;
        }
        rewards.push((faction_id.0, source_amount / SPILLOVER_RATIO));
    }

    if rewards.is_empty() {
        return None;
    }
    // Faction ids are integers: total_cmp orders them like `a[0] - b[0]`.
    rewards.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(rewards)
}

/// `JSON.stringify(a) === JSON.stringify(b)` for merged rewards: undefined alike, numbers alike
/// when they print alike (±0 as 0, NaN and ±Infinity as null).
fn json_equal(a: &Option<QuestReputationRewardValue>, b: &Option<QuestReputationRewardValue>) -> bool {
    let number_equal = |x: f64, y: f64| {
        if x.is_finite() && y.is_finite() {
            x == y
        } else {
            !x.is_finite() && !y.is_finite()
        }
    };
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| number_equal(x.0, y.0) && number_equal(x.1, y.1)),
        _ => false,
    }
}

/// The quest's reputation reward; None when no reliable reward could be derived.
pub fn merge(observations: &[Observation<ReputationTurnInValue>]) -> Option<QuestReputationRewardValue> {
    let of_faction = |faction: Faction| observations.iter().filter(|obs| obs.value.player_faction == faction).collect::<Vec<_>>();
    let (alliance_observations, horde_observations) = (of_faction(Faction::Alliance), of_faction(Faction::Horde));
    let alliance = merge_for_player_faction(&alliance_observations);
    let horde = merge_for_player_faction(&horde_observations);
    if !alliance_observations.is_empty() && !horde_observations.is_empty() && !json_equal(&alliance, &horde) {
        return None;
    }
    alliance.or(horde)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{event, packed, session_json, with_session};
    use serde_json::{json, Value as Json};

    fn orc() -> Json {
        packed(&[json!("Orc"), json!("Orc"), json!(2)])
    }

    fn human() -> Json {
        packed(&[json!("Human"), json!("Human"), json!(1)])
    }

    fn horde() -> Json {
        packed(&[json!("Horde"), json!("Horde")])
    }

    fn faction_name(faction_id: u32) -> &'static str {
        match faction_id {
            68 => "Undercity",
            76 => "Orgrimmar",
            87 => "Bloodsail Buccaneers",
            529 => "Argent Dawn",
            _ => unreachable!("no test faction {faction_id}"),
        }
    }

    /// test/fixtures.ts `apiSample(time, preciseTime, value)`; a null value is left out (nil).
    fn api_sample(t: f64, value: Json) -> Json {
        if value.is_null() {
            json!({ "t": t, "tp": t })
        } else {
            json!({ "t": t, "tp": t, "v": value })
        }
    }

    /// The test file's `session(events, standings, race, locale)`.
    fn session(events: Vec<Json>, standings: &[(u32, &[(f64, f64)])], race: Option<Json>, locale: &str) -> Json {
        let mut factions = serde_json::Map::new();
        for &(faction_id, samples) in standings {
            let entries = samples
                .iter()
                .map(|&(t, value)| {
                    api_sample(
                        t,
                        json!({ "name": faction_name(faction_id), "factionID": faction_id, "currentStanding": value }),
                    )
                })
                .collect();
            factions.insert(faction_id.to_string(), Json::Array(entries));
        }
        let race_samples: Vec<Json> = race.into_iter().map(|race| api_sample(0.0, race)).collect();
        session_json(
            json!({
                "GetLocale": { "player": [api_sample(0.0, json!(locale))] },
                "UnitRace": { "player": race_samples },
                "UnitFactionGroup": { "player": [api_sample(0.0, horde())] },
                "C_Reputation.GetFactionDataByID": factions,
            }),
            events,
        )
    }

    fn orc_session(events: Vec<Json>, standings: &[(u32, &[(f64, f64)])]) -> Json {
        session(events, standings, Some(orc()), "enUS")
    }

    fn message(t: f64, text: &str) -> Json {
        event(t, "CHAT_MSG_COMBAT_FACTION_CHANGE", packed(&[json!(text)]))
    }

    fn turn_in(t: f64, quest_id: f64) -> Json {
        event(t, "QUEST_TURNED_IN", packed(&[json!(quest_id), json!(0), json!(0)]))
    }

    fn observed(session: &Json) -> Vec<Observation<ReputationTurnInValue>> {
        with_session(session, observe)
    }

    fn values(session: &Json) -> Vec<ReputationTurnInValue> {
        observed(session).into_iter().map(|o| o.value).collect()
    }

    fn turn_in_value<const G: usize, const S: usize>(gains: [(f64, f64); G], spillover: [(f64, f64); S]) -> ReputationTurnInValue {
        ReputationTurnInValue {
            player_faction: Faction::Horde,
            gains: gains.into(),
            spillover: spillover.into(),
        }
    }

    #[test]
    fn records_reputation_changes_around_a_quest_turn_in() {
        let session = orc_session(
            vec![turn_in(10.0, 99001.0)],
            &[
                (68, &[(0.0, 3000.0), (9.9, 3100.0)]),
                (76, &[(0.0, 5000.0)]),
                (87, &[(0.0, 400.0), (10.1, 375.0)]),
            ],
        );
        let observed: Vec<_> = observed(&session).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect();
        assert_eq!(observed, [(99001, turn_in_value([(68.0, 100.0), (87.0, -25.0)], []), Confidence::High, 10.0)]);
    }

    fn legacy_sample(t: f64, name: &str, bar_value: f64) -> Json {
        api_sample(t, json!({ "n": 14, "1": name, "3": 5, "4": 3000, "5": 9000, "6": bar_value, "14": 68 }))
    }

    #[test]
    fn reads_the_bar_value_of_the_legacy_faction_api() {
        let session = session_json(
            json!({
                "UnitRace": { "player": [api_sample(0.0, orc())] },
                "UnitFactionGroup": { "player": [api_sample(0.0, horde())] },
                "GetFactionInfoByID": { "68": [legacy_sample(0.0, "Undercity", 3000.0), legacy_sample(10.0, "Undercity", 3250.0)] },
            }),
            vec![turn_in(10.0, 99001.0)],
        );
        assert_eq!(values(&session), [turn_in_value([(68.0, 250.0)], [])]);
    }

    #[test]
    fn falls_back_to_the_legacy_faction_api_when_modern_samples_have_no_standing() {
        let session = session_json(
            json!({
                "GetLocale": { "player": [api_sample(0.0, json!("deDE"))] },
                "UnitRace": { "player": [api_sample(0.0, orc())] },
                "UnitFactionGroup": { "player": [api_sample(0.0, horde())] },
                "C_Reputation.GetFactionDataByID": { "68": [api_sample(0.0, Json::Null)] },
                "GetFactionInfoByID": { "68": [legacy_sample(0.0, "Unterstadt", 3000.0), legacy_sample(10.0, "Unterstadt", 3250.0)] },
            }),
            vec![
                turn_in(10.0, 99001.0),
                message(10.0, "Euer Ruf bei der Fraktion Unterstadt hat sich um 250 verbessert."),
            ],
        );
        assert_eq!(values(&session), [turn_in_value([(68.0, 250.0)], [])]);
    }

    #[test]
    fn splits_off_spillover_gains() {
        let session = orc_session(
            vec![turn_in(10.0, 99001.0)],
            &[(68, &[(0.0, 3000.0), (10.0, 3100.0)]), (76, &[(0.0, 5000.0), (10.0, 5026.0)])],
        );
        assert_eq!(values(&session), [turn_in_value([(68.0, 100.0)], [(76.0, 68.0)])]);
    }

    #[test]
    fn ignores_changes_outside_the_turn_in_window() {
        let session = orc_session(vec![turn_in(10.0, 99001.0)], &[(68, &[(0.0, 3000.0), (9.4, 3050.0), (10.8, 3100.0)])]);
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn reads_the_standing_strictly_before_the_window_and_up_to_its_end_inclusive() {
        // Not in the TS tests. Window [9.5, 10.5]: the 9.5 sample is inside it, the 10.5 one too.
        let session = orc_session(vec![turn_in(10.0, 99001.0)], &[(68, &[(0.0, 3000.0), (9.5, 3050.0), (10.5, 3100.0)])]);
        assert_eq!(values(&session), [turn_in_value([(68.0, 100.0)], [])]);
    }

    #[test]
    fn skips_turn_ins_close_to_another_turn_in() {
        let session = orc_session(vec![turn_in(10.0, 99001.0), turn_in(10.6, 99002.0)], &[(68, &[(0.0, 3000.0), (10.0, 3100.0)])]);
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn uses_the_chat_messages_when_a_faction_is_first_seen_in_the_turn_in_window() {
        let session = orc_session(
            vec![
                message(9.9, "Reputation with Argent Dawn increased by 150."),
                turn_in(10.0, 99001.0),
                message(10.1, "Reputation with Bloodsail Buccaneers decreased by 25."),
            ],
            &[(87, &[(0.0, 400.0), (10.1, 375.0)]), (529, &[(10.0, 350.0)])],
        );
        assert_eq!(values(&session), [turn_in_value([(87.0, -25.0), (529.0, 150.0)], [])]);
    }

    #[test]
    fn uses_the_chat_messages_when_a_gaining_faction_has_no_reputation_stream_yet() {
        let session = orc_session(
            vec![
                turn_in(10.0, 99001.0),
                message(10.1, "Reputation with Undercity increased by 50."),
                message(10.1, "Reputation with Argent Dawn increased by 100."),
            ],
            &[(68, &[(0.0, 3000.0), (10.1, 3050.0)]), (529, &[(400.0, 100.0)])],
        );
        assert_eq!(values(&session), [turn_in_value([(68.0, 50.0), (529.0, 100.0)], [])]);
    }

    #[test]
    fn skips_turn_ins_with_more_reputation_messages_than_changes_outside_en_us() {
        let session = session(
            vec![
                turn_in(10.0, 99001.0),
                message(10.1, "Euer Ruf bei der Fraktion Unterstadt hat sich um 50 verbessert."),
                message(10.1, "Euer Ruf bei der Fraktion Der Irdene Ring hat sich um 100 verbessert."),
            ],
            &[(68, &[(0.0, 3000.0), (10.1, 3050.0)])],
            Some(orc()),
            "deDE",
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn does_not_count_reputation_messages_without_an_amount_as_changes() {
        let session = session(
            vec![
                turn_in(10.0, 99001.0),
                message(10.1, "Euer Ruf bei der Fraktion Unterstadt hat sich um 50 verbessert."),
                message(10.1, "Ihr seid jetzt Freundlich mit der Fraktion Unterstadt."),
            ],
            &[(68, &[(0.0, 2950.0), (10.1, 3000.0)])],
            Some(orc()),
            "deDE",
        );
        assert_eq!(values(&session), [turn_in_value([(68.0, 50.0)], [])]);
    }

    #[test]
    fn skips_turn_ins_with_a_faction_first_seen_in_the_window_outside_en_us() {
        let session = session(
            vec![
                turn_in(10.0, 99001.0),
                message(10.0, "Euer Ruf bei der Fraktion Argentumdämmerung hat sich um 150 verbessert."),
            ],
            &[(529, &[(10.0, 350.0)])],
            Some(orc()),
            "deDE",
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn skips_turn_ins_with_a_chat_faction_name_missing_from_the_reputation_streams() {
        let session = orc_session(
            vec![turn_in(10.0, 99001.0), message(10.0, "Reputation with Unknown Faction increased by 100.")],
            &[(529, &[(10.0, 350.0)])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn skips_human_players() {
        let session = session(vec![turn_in(10.0, 99001.0)], &[(68, &[(0.0, 3000.0), (10.0, 3110.0)])], Some(human()), "enUS");
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn skips_sessions_without_a_known_race() {
        let session = session(vec![turn_in(10.0, 99001.0)], &[(68, &[(0.0, 3000.0), (10.0, 3100.0)])], None, "enUS");
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn records_turn_ins_of_non_en_us_sessions() {
        let session = session(vec![turn_in(10.0, 99001.0)], &[(68, &[(0.0, 3000.0), (10.0, 3100.0)])], Some(orc()), "deDE");
        assert_eq!(values(&session), [turn_in_value([(68.0, 100.0)], [])]);
    }

    #[test]
    fn parses_reputation_messages_like_the_ts_regex() {
        assert_eq!(
            parse_reputation_message("Reputation with Argent Dawn increased by 150."),
            Some(("Argent Dawn", 150.0))
        );
        // (.+) is greedy: the name runs to the last " increased by N."
        assert_eq!(
            parse_reputation_message("Reputation with A decreased by 5. increased by 7."),
            Some(("A decreased by 5.", 7.0))
        );
        // (.+) needs a character between "with " and " increased".
        assert_eq!(parse_reputation_message("Reputation with  increased by 7."), None);
        assert_eq!(parse_reputation_message("Reputation with X increased by 7"), None);
        assert_eq!(parse_reputation_message("Reputation with X increased by ."), None);
        assert_eq!(parse_reputation_message("Reputation with X\nY increased by 7."), None);
    }

    #[test]
    fn split_spillover_keeps_gains_that_are_not_a_quarter_of_another() {
        let all = FactionRecord::from([(68.0, 100.0), (76.0, 50.0)]);
        assert_eq!(
            split_spillover(&all),
            (FactionRecord::from([(68.0, 100.0), (76.0, 50.0)]), FactionRecord::default())
        );
    }

    #[test]
    fn split_spillover_splits_off_gains_rounded_either_way_from_a_quarter() {
        let all = FactionRecord::from([(69.0, 75.0), (47.0, 18.0), (54.0, 19.0)]);
        assert_eq!(
            split_spillover(&all),
            (FactionRecord::from([(69.0, 75.0)]), FactionRecord::from([(47.0, 69.0), (54.0, 69.0)]))
        );
    }

    #[test]
    fn split_spillover_picks_the_largest_matching_gain_as_the_source() {
        let all = FactionRecord::from([(68.0, 96.0), (69.0, 100.0), (47.0, 25.0)]);
        assert_eq!(
            split_spillover(&all),
            (FactionRecord::from([(68.0, 96.0), (69.0, 100.0)]), FactionRecord::from([(47.0, 69.0)]))
        );
    }

    #[test]
    fn split_spillover_does_not_treat_a_gain_in_the_other_direction_as_spillover() {
        let all = FactionRecord::from([(68.0, 100.0), (87.0, -25.0)]);
        assert_eq!(
            split_spillover(&all),
            (FactionRecord::from([(68.0, 100.0), (87.0, -25.0)]), FactionRecord::default())
        );
        // Not in the TS test, where -25 is also too far from 100 / 4: only the direction rules
        // out -0.5 as spillover of 2 (it can only decide for fractional amounts).
        let all = FactionRecord::from([(68.0, 2.0), (87.0, -0.5)]);
        assert_eq!(
            split_spillover(&all),
            (FactionRecord::from([(68.0, 2.0), (87.0, -0.5)]), FactionRecord::default())
        );
    }

    #[test]
    fn faction_records_iterate_in_js_property_order() {
        let record = FactionRecord::from([(87.0, 1.0), (-5.0, 2.0), (68.0, 3.0), (5e9, 4.0), (-0.0, 5.0)]);
        assert_eq!(
            record.iter().collect::<Vec<_>>(),
            [(0.0, 5.0), (68.0, 3.0), (87.0, 1.0), (-5.0, 2.0), (5e9, 4.0)]
        );
    }

    fn vote<const G: usize, const S: usize>(
        session: u32,
        faction: Faction,
        gains: [(f64, f64); G],
        spillover: [(f64, f64); S],
    ) -> Observation<ReputationTurnInValue> {
        let value = ReputationTurnInValue {
            player_faction: faction,
            gains: gains.into(),
            spillover: spillover.into(),
        };
        Observation {
            entity_id: 1,
            value,
            confidence: Confidence::High,
            t: 0.0,
            session: SessionId(session),
        }
    }

    const H: Faction = Faction::Horde;
    const A: Faction = Faction::Alliance;

    #[test]
    fn merge_returns_the_reward_agreed_on_by_at_least_two_sessions() {
        assert_eq!(
            merge(&[vote(1, H, [(68.0, 100.0)], []), vote(2, H, [(68.0, 100.0)], [])]),
            Some(vec![(68.0, 100.0)])
        );
    }

    #[test]
    fn merge_returns_nothing_when_only_one_session_was_observed() {
        assert_eq!(merge(&[vote(1, H, [(68.0, 100.0)], []), vote(1, H, [(68.0, 100.0)], [])]), None);
    }

    #[test]
    fn merge_tolerates_a_minority_of_bonus_amounts() {
        let mut observations: Vec<_> = (1..=4).map(|session| vote(session, H, [(68.0, 100.0)], [])).collect();
        observations.push(vote(5, H, [(68.0, 110.0)], []));
        assert_eq!(merge(&observations), Some(vec![(68.0, 100.0)]));
    }

    #[test]
    fn merge_returns_nothing_when_amounts_disagree_too_often() {
        assert_eq!(
            merge(&[vote(1, H, [(68.0, 100.0)], []), vote(2, H, [(68.0, 100.0)], []), vote(3, H, [(68.0, 50.0)], [])]),
            None
        );
    }

    #[test]
    fn merge_drops_a_faction_that_shows_up_in_less_than_half_of_the_turn_ins() {
        let observations = [
            vote(1, H, [(68.0, 100.0), (87.0, 5.0)], []),
            vote(2, H, [(68.0, 100.0), (87.0, 5.0)], []),
            vote(3, H, [(68.0, 100.0)], []),
            vote(4, H, [(68.0, 100.0)], []),
            vote(5, H, [(68.0, 100.0)], []),
        ];
        assert_eq!(merge(&observations), Some(vec![(68.0, 100.0)]));
    }

    #[test]
    fn merge_returns_nothing_when_alliance_and_horde_rewards_differ() {
        let observations = [
            vote(1, A, [(2778.0, 50.0)], []),
            vote(2, A, [(2778.0, 50.0)], []),
            vote(3, H, [(2779.0, 50.0)], []),
            vote(4, H, [(2779.0, 50.0)], []),
        ];
        assert_eq!(merge(&observations), None);
    }

    #[test]
    fn merge_returns_the_shared_reward_when_alliance_and_horde_agree() {
        let observations = [
            vote(1, A, [(59.0, 75.0)], []),
            vote(2, A, [(59.0, 75.0)], []),
            vote(3, H, [(59.0, 75.0)], []),
            vote(4, H, [(59.0, 75.0)], []),
        ];
        assert_eq!(merge(&observations), Some(vec![(59.0, 75.0)]));
    }

    #[test]
    fn merge_adds_spillover_as_an_unrounded_quarter_of_its_decided_source_reward() {
        let observations = [
            vote(1, A, [(69.0, 75.0)], [(47.0, 69.0), (54.0, 69.0)]),
            vote(2, A, [(69.0, 75.0)], [(47.0, 69.0), (54.0, 69.0)]),
            vote(3, A, [(69.0, 75.0)], [(47.0, 69.0)]),
        ];
        assert_eq!(merge(&observations), Some(vec![(47.0, 18.75), (54.0, 18.75), (69.0, 75.0)]));
    }

    #[test]
    fn merge_keeps_the_sign_of_negative_spillover() {
        let observations = [vote(1, H, [(87.0, -150.0)], [(21.0, 87.0)]), vote(2, H, [(87.0, -150.0)], [(21.0, 87.0)])];
        assert_eq!(merge(&observations), Some(vec![(21.0, -37.5), (87.0, -150.0)]));
    }

    #[test]
    fn merge_prefers_a_decided_base_reward_over_spillover_for_the_same_faction() {
        let observations = [
            vote(1, A, [(69.0, 100.0), (47.0, 25.0)], []),
            vote(2, A, [(69.0, 100.0), (47.0, 25.0)], []),
            vote(3, A, [(69.0, 100.0)], [(47.0, 69.0)]),
        ];
        assert_eq!(merge(&observations), Some(vec![(47.0, 25.0), (69.0, 100.0)]));
    }

    #[test]
    fn merge_drops_spillover_seen_in_only_one_session() {
        let observations = [vote(1, A, [(69.0, 100.0)], [(47.0, 69.0)]), vote(2, A, [(69.0, 100.0)], [])];
        assert_eq!(merge(&observations), Some(vec![(69.0, 100.0)]));
    }

    #[test]
    fn merge_drops_spillover_whose_source_reward_was_not_decided() {
        let observations = [vote(1, A, [(69.0, 100.0)], [(47.0, 69.0)]), vote(2, A, [(69.0, 50.0)], [(47.0, 69.0)])];
        assert_eq!(merge(&observations), None);
    }

    #[test]
    fn merge_sorts_rewards_by_faction_id() {
        let observations = [vote(1, H, [(87.0, -25.0), (68.0, 100.0)], []), vote(2, H, [(87.0, -25.0), (68.0, 100.0)], [])];
        assert_eq!(merge(&observations), Some(vec![(68.0, 100.0), (87.0, -25.0)]));
    }
}
