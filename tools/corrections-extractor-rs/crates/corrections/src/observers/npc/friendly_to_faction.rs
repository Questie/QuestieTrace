//! npcKeys.friendlyToFaction - Questie field 13. Port of `extract/observers/npc/friendlyToFaction.ts`.
//!
//! "A" and/or "H" for the factions the NPC is friendly to; nil (Questie's default) when hostile
//! to both. Questie treats neutral NPCs (wildlife) as friendly, so a faction counts as friendly
//! when UnitReaction("player", npc) is 4 (neutral) or higher.
//!
//! Source: UnitReaction["player"][token] from Modules/Trackers/UnitState.lua, paired with the
//! NPC's GUID. A wrong pairing is possible; the strict merge guards against it.
//!
//! The merge only decides a faction when at least MIN_SESSIONS_PER_FACTION sessions agree and
//! none disagrees, and only exports a value when both factions are decided. The one exception is
//! a fix by Blizzard: when sessions ordered by server time switch from one reaction to the other
//! exactly once, the newer reaction wins if at least MIN_SESSIONS_PER_FACTION sessions saw it.
//! Any other disagreement (several switches, a switch between sessions with the same server time,
//! a session that saw both, a session without a server time) leaves the faction undecided.

use crate::aggregate::Num;
use crate::guid::{parse_guid, GuidKind};
use crate::observation::{Confidence, Observation, SessionId};
use crate::probes::{player_faction_at, session_server_time, Faction};
use indexmap::{IndexMap, IndexSet};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use trace_data::stream::{entry_t, entry_v, get_stream, value_at};
use trace_data::{KeyRef, Session, Value};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FactionReactionValue {
    pub faction: Faction,
    pub friendly: bool,
    /// GetServerTime() at the start of the session, to order sessions; None when not recorded.
    pub session_time: Option<f64>,
}

const MIN_SESSIONS_PER_FACTION: usize = 2;
const FRIENDLY_REACTION_MIN: f64 = 4.0;

pub fn observe(session: &Session) -> Vec<Observation<FactionReactionValue>> {
    let mut observations = Vec::new();
    // `${npcID}:${faction}:${friendly}`
    let mut seen: HashSet<(Num, Faction, bool)> = HashSet::new();
    let session_time = session_server_time(session);
    for encounter in reaction_encounters(session) {
        let Some(faction) = player_faction_at(session, encounter.t) else { continue };
        let Some(reaction) = encounter.value.as_num() else { continue };
        let friendly = reaction >= FRIENDLY_REACTION_MIN;
        if !seen.insert((Num(encounter.npc_id), faction, friendly)) {
            continue;
        }
        let value = FactionReactionValue { faction, friendly, session_time };
        observations.push(Observation::new(encounter.npc_id as i64, value, Confidence::High, encounter.t));
    }
    observations
}

/// The faction's reaction agreed by the sessions, or None when undecided.
fn decide_faction(observations: &[Observation<FactionReactionValue>], faction: Faction) -> Option<bool> {
    struct SessionReactions {
        friendly: IndexSet<bool>,
        time: Option<f64>,
    }
    let mut sessions: IndexMap<SessionId, SessionReactions> = IndexMap::new();
    for obs in observations.iter().filter(|obs| obs.value.faction == faction) {
        let session = sessions.entry(obs.session).or_insert_with(|| SessionReactions { friendly: IndexSet::new(), time: obs.value.session_time });
        session.friendly.insert(obs.value.friendly);
    }
    if sessions.values().any(|session| session.friendly.len() > 1) {
        return None;
    }

    let first = sessions.values().next().map(|session| session.friendly[0]);
    if sessions.values().all(|session| Some(session.friendly[0]) == first) {
        return first.filter(|_| sessions.len() >= MIN_SESSIONS_PER_FACTION);
    }

    struct Vote {
        friendly: bool,
        time: f64,
    }
    // Sessions disagree: only a single switch over server time can decide, so every session needs one.
    let mut votes: Vec<Vote> = sessions.values().map(|session| Some(Vote { friendly: session.friendly[0], time: session.time? })).collect::<Option<_>>()?;
    votes.sort_by(|a, b| by_server_time(a.time, b.time));
    let switches: Vec<usize> = (1..votes.len()).filter(|&i| votes[i].friendly != votes[i - 1].friendly).collect();
    if switches.len() != 1 || votes[switches[0]].time == votes[switches[0] - 1].time {
        return None;
    }
    let newest = votes[votes.len() - 1].friendly;
    let newest_count = votes.iter().filter(|vote| vote.friendly == newest).count();
    (newest_count >= MIN_SESSIONS_PER_FACTION).then_some(newest)
}

/// TS sorts by `a.time - b.time`. That order for numbers (-0 equal to 0); NaN, which server
/// times never are, goes last instead of breaking the total order Rust's sort requires.
fn by_server_time(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or_else(|| a.is_nan().cmp(&b.is_nan()))
}

/// "A", "H" or "AH"; None when the value is not reliable enough to export or the NPC is hostile
/// to both factions.
pub fn merge(observations: &[Observation<FactionReactionValue>]) -> Option<&'static str> {
    let alliance = decide_faction(observations, Faction::Alliance)?;
    let horde = decide_faction(observations, Faction::Horde)?;
    match (alliance, horde) {
        (true, true) => Some("AH"),
        (true, false) => Some("A"),
        (false, true) => Some("H"),
        (false, false) => None,
    }
}

// --- UnitReaction paired with NPC GUIDs ---
//
// A private port of `_unitState.ts` `unitStateEncounters(session, "UnitReaction", "player")`
// and the npc part of `_guidEncounters.ts`, which are not ported yet (group C). Replace it with
// `npc::unit_state` once that lands.
//
// Both the UnitGUID and the UnitState streams are only written on change, so a value is only
// trusted at a GUID sample recorded at the same time as the value, or when PLAYER_TARGET_CHANGED
// or UPDATE_MOUSEOVER_UNIT fired at that time (UnitState samples every token on both, so the
// last value is still current).

const UNIT_STATE_TOKENS: [&str; 2] = ["target", "mouseover"];
const UNIT_STATE_SAMPLE_EVENTS: [&str; 2] = ["PLAYER_TARGET_CHANGED", "UPDATE_MOUSEOVER_UNIT"];

struct ReactionEncounter<'v, 'a> {
    npc_id: f64,
    t: f64,
    value: &'v Value<'a>,
}

fn entry_tp(entry: &Value) -> f64 {
    entry.get("tp").and_then(Value::as_num).unwrap_or(f64::NAN)
}

/// emulator.ts `valueAt` over a filtered stream.
fn value_at_entries<'v, 'a>(entries: &[&'v Value<'a>], target: f64) -> Option<&'v Value<'a>> {
    let (mut lo, mut hi) = (0isize, entries.len() as isize - 1);
    let mut result = None;
    while lo <= hi {
        let mid = ((lo + hi) as usize) >> 1;
        if entry_t(entries[mid]) <= target {
            result = Some(mid);
            lo = mid as isize + 1;
        } else {
            hi = mid as isize - 1;
        }
    }
    entry_v(entries[result?])
}

fn reaction_encounters<'v, 'a>(session: &Session<'v, 'a>) -> Vec<ReactionEncounter<'v, 'a>> {
    let mut out = Vec::new();
    let sample_event_times: Vec<f64> =
        session.events().filter(|event| event.name().is_some_and(|name| UNIT_STATE_SAMPLE_EVENTS.contains(&name))).map(|event| event.t()).collect();
    for token in UNIT_STATE_TOKENS {
        let stream = get_stream(session, "UnitReaction", &[KeyRef::Str("player"), KeyRef::Str(token)]).unwrap_or(&[]);
        let guid_stream = get_stream(session, "UnitGUID", &[KeyRef::Str(token)]).unwrap_or(&[]);
        let mut guid_tps: HashMap<Num, Vec<f64>> = HashMap::new();
        for entry in guid_stream {
            guid_tps.entry(Num(entry_t(entry))).or_default().push(entry_tp(entry));
        }
        let sample_times: HashSet<Num> = sample_event_times.iter().copied().chain(stream.iter().map(entry_t)).map(Num).collect();

        for entry in guid_stream {
            let Some(guid) = entry_v(entry).and_then(Value::as_str) else { continue };
            let parsed = parse_guid(Some(guid));
            let (GuidKind::Npc, Some(npc_id)) = (parsed.kind, parsed.id) else { continue };
            let (t, tp) = (entry_t(entry), entry_tp(entry));
            if !sample_times.contains(&Num(t)) {
                continue;
            }
            // A later GUID sample in the same frame starts another unit, so its value must not be
            // used. Relies on UnitInteraction.lua sampling the GUID before UnitState.lua samples
            // the value (.toc order).
            let next_tp = guid_tps.get(&Num(t)).and_then(|tps| tps.iter().copied().find(|&other| other > tp));
            let value = match next_tp {
                None => value_at(stream, t),
                Some(next_tp) => {
                    let before_next: Vec<&Value> = stream.iter().filter(|e| entry_t(e) < t || entry_tp(e) < next_tp).collect();
                    value_at_entries(&before_next, t)
                }
            };
            let Some(value) = value else { continue };
            out.push(ReactionEncounter { npc_id, t, value });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{event, packed, session_json, stream, with_session};
    use serde_json::{json, Value as Json};
    use Faction::{Alliance, Horde};

    fn alliance() -> Json {
        json!({ "n": 2, "1": "Alliance", "2": "Alliance" })
    }

    fn horde() -> Json {
        json!({ "n": 2, "1": "Horde", "2": "Horde" })
    }

    fn creature_guid(id: u32) -> Json {
        json!(format!("Creature-0-5208-0-7-{id}-000032"))
    }

    /// One stream entry per (t, v); a null `v` records nil.
    fn samples(entries: &[(f64, Json)]) -> Json {
        stream(entries.iter().cloned())
    }

    fn reaction(faction: Faction, friendly: bool, session_time: Option<f64>) -> FactionReactionValue {
        FactionReactionValue { faction, friendly, session_time }
    }

    fn observed(session: &Json) -> Vec<(i64, FactionReactionValue, Confidence, f64)> {
        with_session(session, |s| observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect())
    }

    /// TS `vote(entityId, sessionKey, faction, friendly, sessionTime = sessionKey * 1000)`.
    fn vote(session: u32, faction: Faction, friendly: bool) -> Observation<FactionReactionValue> {
        vote_at(session, faction, friendly, Some(f64::from(session) * 1000.0))
    }

    fn vote_at(session: u32, faction: Faction, friendly: bool, session_time: Option<f64>) -> Observation<FactionReactionValue> {
        Observation { entity_id: 1, value: reaction(faction, friendly, session_time), confidence: Confidence::High, t: 0.0, session: SessionId(session) }
    }

    #[test]
    fn pairs_a_reaction_recorded_at_the_same_time_as_the_target_guid() {
        let session = session_json(
            json!({
                "UnitFactionGroup": { "player": samples(&[(0.0, alliance())]) },
                "UnitGUID": { "target": samples(&[(5.0, creature_guid(200001))]) },
                "UnitReaction": { "player": { "target": samples(&[(5.0, json!(4))]) } },
            }),
            vec![],
        );
        assert_eq!(observed(&session), [(200001, reaction(Alliance, true, None), Confidence::High, 5.0)]);
    }

    #[test]
    fn uses_an_unchanged_reaction_when_the_token_change_event_fired_at_the_guid_sample() {
        let session = session_json(
            json!({
                "UnitFactionGroup": { "player": samples(&[(0.0, horde())]) },
                "UnitGUID": { "mouseover": samples(&[(3.0, creature_guid(200001)), (8.0, creature_guid(200002))]) },
                "UnitReaction": { "player": { "mouseover": samples(&[(3.0, json!(2))]) } },
            }),
            vec![event(3.0, "UPDATE_MOUSEOVER_UNIT", packed(&[])), event(8.0, "UPDATE_MOUSEOVER_UNIT", packed(&[]))],
        );
        let found: Vec<(i64, FactionReactionValue)> = observed(&session).into_iter().map(|o| (o.0, o.1)).collect();
        assert_eq!(found, [(200001, reaction(Horde, false, None)), (200002, reaction(Horde, false, None))]);
    }

    #[test]
    fn does_not_pair_a_stale_reaction_with_a_guid_sampled_at_another_event() {
        let session = session_json(
            json!({
                "UnitFactionGroup": { "player": samples(&[(0.0, alliance())]) },
                "UnitGUID": { "target": samples(&[(3.0, creature_guid(200001)), (8.0, creature_guid(200002))]) },
                "UnitReaction": { "player": { "target": samples(&[(3.0, json!(5))]) } },
            }),
            vec![event(3.0, "PLAYER_TARGET_CHANGED", packed(&[])), event(8.0, "QUEST_DETAIL", packed(&[]))],
        );
        let npcs: Vec<i64> = observed(&session).into_iter().map(|o| o.0).collect();
        assert_eq!(npcs, [200001]);
    }

    #[test]
    fn treats_neutral_as_friendly_and_unfriendly_as_hostile() {
        let session = session_json(
            json!({
                "UnitFactionGroup": { "player": samples(&[(0.0, alliance())]) },
                "UnitGUID": { "target": samples(&[(3.0, creature_guid(200001)), (5.0, creature_guid(200002))]) },
                "UnitReaction": { "player": { "target": samples(&[(3.0, json!(3)), (5.0, json!(4))]) } },
            }),
            vec![],
        );
        let found: Vec<(i64, bool)> = observed(&session).into_iter().map(|o| (o.0, o.1.friendly)).collect();
        assert_eq!(found, [(200001, false), (200002, true)]);
    }

    #[test]
    fn uses_either_unit_change_event_to_pair_a_reaction_for_both_tokens() {
        let session = session_json(
            json!({
                "UnitFactionGroup": { "player": samples(&[(0.0, alliance())]) },
                "UnitGUID": { "target": samples(&[(3.0, creature_guid(200001)), (8.0, creature_guid(200002))]) },
                "UnitReaction": { "player": { "target": samples(&[(3.0, json!(5))]) } },
            }),
            vec![event(8.0, "UPDATE_MOUSEOVER_UNIT", packed(&[]))],
        );
        let npcs: Vec<i64> = observed(&session).into_iter().map(|o| o.0).collect();
        assert_eq!(npcs, [200001, 200002]);
    }

    // Not in friendlyToFaction.test.ts: covers the private unit-state pairing, mirroring
    // level.test.ts "should pair each guid with its own level when the target changed twice at
    // the same time".
    #[test]
    fn pairs_each_guid_with_its_own_reaction_when_the_target_changed_twice_at_the_same_time() {
        let sample = |tp: f64, v: Json| json!({ "t": 5, "tp": tp, "v": v });
        let session = session_json(
            json!({
                "UnitFactionGroup": { "player": samples(&[(0.0, alliance())]) },
                "UnitGUID": { "target": [sample(5.1, creature_guid(200001)), sample(5.3, creature_guid(200002))] },
                "UnitReaction": { "player": { "target": [sample(5.2, json!(3)), sample(5.4, json!(5))] } },
            }),
            vec![],
        );
        let found: Vec<(i64, bool)> = observed(&session).into_iter().map(|o| (o.0, o.1.friendly)).collect();
        assert_eq!(found, [(200001, false), (200002, true)]);
    }

    #[test]
    fn records_one_observation_per_npc_faction_and_reaction_kind_in_a_session() {
        let session = session_json(
            json!({
                "UnitFactionGroup": { "player": samples(&[(0.0, alliance())]) },
                "UnitGUID": { "target": samples(&[(3.0, creature_guid(200001)), (4.0, Json::Null), (5.0, creature_guid(200001))]) },
                "UnitReaction": { "player": { "target": samples(&[(3.0, json!(5)), (5.0, json!(6))]) } },
            }),
            vec![],
        );
        assert_eq!(observed(&session).len(), 1);
    }

    #[test]
    fn records_the_first_server_time_of_the_session() {
        let session = session_json(
            json!({
                "GetServerTime": samples(&[(0.0, json!(1790000000)), (50.0, json!(1790000050))]),
                "UnitFactionGroup": { "player": samples(&[(0.0, alliance())]) },
                "UnitGUID": { "target": samples(&[(5.0, creature_guid(200001))]) },
                "UnitReaction": { "player": { "target": samples(&[(5.0, json!(4))]) } },
            }),
            vec![],
        );
        let times: Vec<Option<f64>> = observed(&session).into_iter().map(|o| o.1.session_time).collect();
        assert_eq!(times, [Some(1790000000.0)]);
    }

    #[test]
    fn skips_encounters_without_a_known_player_faction() {
        let session = session_json(
            json!({
                "UnitGUID": { "target": samples(&[(5.0, creature_guid(200001))]) },
                "UnitReaction": { "player": { "target": samples(&[(5.0, json!(4))]) } },
            }),
            vec![],
        );
        assert_eq!(observed(&session), []);
    }

    // describe("mergeFriendlyToFaction")

    #[test]
    fn merge_gives_ah_when_both_factions_are_friendly_in_at_least_two_sessions_each() {
        assert_eq!(merge(&[vote(1, Alliance, true), vote(2, Alliance, true), vote(3, Horde, true), vote(4, Horde, true)]), Some("AH"));
    }

    #[test]
    fn merge_gives_only_the_friendly_faction_when_the_other_is_hostile() {
        assert_eq!(merge(&[vote(1, Alliance, false), vote(2, Alliance, false), vote(3, Horde, true), vote(4, Horde, true)]), Some("H"));
    }

    #[test]
    fn merge_gives_nothing_when_the_npc_is_hostile_to_both_factions() {
        assert_eq!(merge(&[vote(1, Alliance, false), vote(2, Alliance, false), vote(3, Horde, false), vote(4, Horde, false)]), None);
    }

    #[test]
    fn merge_gives_nothing_when_only_one_faction_was_observed() {
        assert_eq!(merge(&[vote(1, Alliance, true), vote(2, Alliance, true)]), None);
    }

    #[test]
    fn merge_gives_nothing_when_a_faction_was_observed_in_only_one_session() {
        assert_eq!(merge(&[vote(1, Alliance, true), vote(1, Alliance, true), vote(3, Horde, true), vote(4, Horde, true)]), None);
    }

    #[test]
    fn merge_gives_nothing_when_sessions_disagree_for_a_faction() {
        let votes = [vote(1, Alliance, true), vote(2, Alliance, true), vote(5, Alliance, false), vote(3, Horde, true), vote(4, Horde, true)];
        assert_eq!(merge(&votes), None);
    }

    #[test]
    fn merge_uses_the_newer_reaction_when_sessions_switch_once_over_time() {
        let votes = [vote(1, Alliance, false), vote(2, Alliance, true), vote(3, Alliance, true), vote(4, Horde, true), vote(5, Horde, true)];
        assert_eq!(merge(&votes), Some("AH"));
    }

    #[test]
    fn merge_decides_without_server_times_when_all_sessions_agree() {
        let votes = [vote_at(1, Alliance, true, None), vote_at(2, Alliance, true, None), vote_at(3, Horde, true, None), vote_at(4, Horde, true, None)];
        assert_eq!(merge(&votes), Some("AH"));
    }

    #[test]
    fn merge_gives_nothing_when_the_switch_happens_between_sessions_with_the_same_server_time() {
        let votes = [
            vote_at(1, Alliance, false, Some(1000.0)),
            vote_at(2, Alliance, true, Some(1000.0)),
            vote_at(3, Alliance, true, Some(2000.0)),
            vote(4, Horde, true),
            vote(5, Horde, true),
        ];
        assert_eq!(merge(&votes), None);
    }

    #[test]
    fn merge_orders_sessions_by_server_time_rather_than_by_session() {
        let votes = [
            vote_at(1, Horde, true, Some(4000.0)),
            vote_at(2, Horde, true, Some(3000.0)),
            vote_at(3, Horde, false, Some(2000.0)),
            vote_at(4, Horde, false, Some(1000.0)),
            vote(5, Alliance, true),
            vote(6, Alliance, true),
        ];
        assert_eq!(merge(&votes), Some("AH"));
    }

    #[test]
    fn merge_gives_nothing_when_the_newer_reaction_was_seen_in_only_one_session() {
        let votes = [vote(1, Alliance, false), vote(2, Alliance, false), vote(3, Alliance, true), vote(4, Horde, true), vote(5, Horde, true)];
        assert_eq!(merge(&votes), None);
    }

    #[test]
    fn merge_gives_nothing_when_sessions_switch_more_than_once_over_time() {
        let votes =
            [vote(1, Alliance, true), vote(2, Alliance, false), vote(3, Alliance, true), vote(4, Alliance, true), vote(5, Horde, true), vote(6, Horde, true)];
        assert_eq!(merge(&votes), None);
    }

    #[test]
    fn merge_gives_nothing_when_disagreeing_sessions_have_no_server_time() {
        let votes = [vote_at(1, Alliance, false, None), vote(2, Alliance, true), vote(3, Alliance, true), vote(4, Horde, true), vote(5, Horde, true)];
        assert_eq!(merge(&votes), None);
    }

    #[test]
    fn merge_gives_nothing_when_one_session_saw_both_reactions() {
        let votes = [
            vote(1, Alliance, false),
            vote(2, Alliance, true),
            vote(2, Alliance, false),
            vote(3, Alliance, true),
            vote(4, Alliance, true),
            vote(5, Horde, true),
            vote(6, Horde, true),
        ];
        assert_eq!(merge(&votes), None);
    }
}
