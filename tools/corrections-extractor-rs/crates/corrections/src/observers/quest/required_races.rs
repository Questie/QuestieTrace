//! questKeys.requiredRaces - Questie field 6 (bitmask, default 0: every race). Port of
//! `extract/observers/quest/requiredRaces.ts`.
//!
//! A wrong mask hides a quest from players who can take it, so a restriction is only exported
//! with proof that a whole faction cannot get the quest:
//! - every starter of the quest is an NPC (items and objects give no reaction to check),
//! - each of them was hostile to that faction in at least MIN_HOSTILE_SESSIONS sessions and
//!   friendly to it in none,
//! - each of them was friendly to the other faction and never hostile to it, which rules out
//!   reputation-based reactions such as Timbermaw's (unfriendly to both factions),
//! - no session of that faction ever saw the quest, while one of the other faction did, and
//! - no session of that faction saw another quest of the same starters. Class quests are given
//!   by a trainer of each faction; that faction seeing the trainer's other quests hints at a
//!   counterpart quest giver that was never traced.
//!
//! Checked against Questie's Classic quest DB: with 3 sessions no quest was restricted wrongly.
//! The exported mask is Questie's ALL_ALLIANCE / ALL_HORDE for Forever, never only the races
//! seen. Remaining risk: a starter of the excluded faction that no trace has seen yet.
//! Race-level restrictions are not inferred (yet). Unfactioned characters are ignored.

use super::encounters::quest_encounters;
use crate::aggregate::Num;
use crate::observation::{Confidence, Observation};
use crate::observers::npc::friendly_to_faction::FactionReactionValue;
use crate::probes::{player_faction_at, Faction};
use indexmap::IndexSet;
use std::collections::HashSet;
use trace_data::stream::{entry_t, entry_v, get_stream};
use trace_data::{KeyRef, Session, Value};

const MIN_HOSTILE_SESSIONS: usize = 3;

// QuestieDB raceKeys.ALL_ALLIANCE / ALL_HORDE for Forever (src/corrections/enum/expansions.lua):
// the Classic races plus Skyborne.
const SKYBORNE_ALLIANCE: u64 = 4294967296;
const SKYBORNE_HORDE: u64 = 8589934592;
pub const ALLIANCE_RACES: u64 = 1 + 4 + 8 + 64 + SKYBORNE_ALLIANCE;
pub const HORDE_RACES: u64 = 2 + 16 + 32 + 128 + SKYBORNE_HORDE;

/// How many distinct sessions saw an NPC hostile and friendly to one faction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FactionReactionCounts {
    pub hostile_sessions: usize,
    pub friendly_sessions: usize,
}

/// TS `ReactionSummary`: the reaction counts of one NPC per faction. The default is
/// `summarizeReactions([])`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ReactionSummary {
    pub alliance: FactionReactionCounts,
    pub horde: FactionReactionCounts,
}

impl ReactionSummary {
    pub fn of(&self, faction: Faction) -> FactionReactionCounts {
        match faction {
            Faction::Alliance => self.alliance,
            Faction::Horde => self.horde,
        }
    }
}

/// QUEST_ACCEPTED's quest id: Era, TBC, Wrath and Mists send (questLogIndex, questID), Forever
/// sends (questID) alone.
fn accepted_quest_id(args: Option<&Value>) -> Option<f64> {
    let args = args.filter(|args| args.is_table())?;
    let value = if args.get("n").and_then(Value::as_num) == Some(1.0) { args.get(1) } else { args.get(2) };
    value?.as_num().filter(|&id| id > 0.0)
}

/// TS `observeQuestFactions`: the player's faction for every quest the session saw, accepted,
/// in the quest log, in a quest frame or in a gossip list. Once per quest and faction.
pub fn observe_quest_factions(session: &Session) -> Vec<Observation<Faction>> {
    // (questID, t)
    let mut sightings: Vec<(f64, f64)> = quest_encounters(session).into_iter().map(|e| (e.quest_id, e.t)).collect();
    for event in session.events() {
        if !event.is("QUEST_ACCEPTED") {
            continue;
        }
        if let Some(quest_id) = accepted_quest_id(event.args()) {
            sightings.push((quest_id, event.t()));
        }
    }
    for entry in get_stream(session, "QuestLog", &[]).unwrap_or(&[]) {
        let Some(quest_ids) = entry_v(entry).and_then(Value::as_array) else { continue };
        for quest_id in quest_ids.iter().filter_map(Value::as_num).filter(|&id| id > 0.0) {
            sightings.push((quest_id, entry_t(entry)));
        }
    }
    for name in ["C_GossipInfo.GetAvailableQuests", "C_GossipInfo.GetActiveQuests"] {
        for entry in get_stream(session, name, &[]).unwrap_or(&[]) {
            let Some(quests) = entry_v(entry).and_then(Value::as_array) else { continue };
            for quest_id in quests.iter().filter_map(|quest| quest.get("questID")?.as_num()).filter(|&id| id > 0.0) {
                sightings.push((quest_id, entry_t(entry)));
            }
        }
    }

    // A sighting before the first faction sample still belongs to the character's faction.
    // Counting it can only cancel a restriction, never cause one.
    let first_faction =
        get_stream(session, "UnitFactionGroup", &[KeyRef::Str("player")]).unwrap_or(&[]).iter().find_map(|entry| player_faction_at(session, entry_t(entry)));
    let mut observations = Vec::new();
    // `${questID}:${faction}`
    let mut seen: HashSet<(Num, Faction)> = HashSet::new();
    for (quest_id, t) in sightings {
        let Some(faction) = player_faction_at(session, t).or(first_faction) else { continue };
        if !seen.insert((Num(quest_id), faction)) {
            continue;
        }
        observations.push(Observation::new(quest_id as i64, faction, Confidence::High, t));
    }
    observations
}

/// TS `mergeQuestFactions`: every faction that saw the quest, in first-seen order.
pub fn merge_quest_factions(observations: &[Observation<Faction>]) -> IndexSet<Faction> {
    observations.iter().map(|obs| obs.value).collect()
}

/// TS `summarizeReactions`: per faction, how many sessions saw the NPC hostile and how many saw
/// it friendly.
pub fn summarize_reactions(observations: &[Observation<FactionReactionValue>]) -> ReactionSummary {
    let mut sessions = HashSet::new();
    for obs in observations {
        sessions.insert((obs.value.faction, obs.value.friendly, obs.session));
    }
    let count = |faction: Faction, friendly: bool| sessions.iter().filter(|&&(f, fr, _)| f == faction && fr == friendly).count();
    let counts = |faction| FactionReactionCounts { hostile_sessions: count(faction, false), friendly_sessions: count(faction, true) };
    ReactionSummary { alliance: counts(Faction::Alliance), horde: counts(Faction::Horde) }
}

fn excluded(faction: Faction, allowed: Faction, seen_factions: &IndexSet<Faction>, starters: &[ReactionSummary], sibling_factions: &IndexSet<Faction>) -> bool {
    if seen_factions.contains(&faction) || sibling_factions.contains(&faction) || !seen_factions.contains(&allowed) {
        return false;
    }
    !starters.is_empty()
        && starters.iter().all(|starter| {
            let (excluded, allowed) = (starter.of(faction), starter.of(allowed));
            excluded.friendly_sessions == 0
                && excluded.hostile_sessions >= MIN_HOSTILE_SESSIONS
                && allowed.friendly_sessions > 0
                && allowed.hostile_sessions == 0
        })
}

/// TS `mergeRequiredRaces`: the race mask to export, or None to keep every race.
///
/// `starters` are the reaction summaries of every NPC starting the quest; pass none when the
/// quest has item or object starters. `sibling_factions` are the factions that saw other quests
/// of the same starters (TS defaults it to an empty set).
pub fn merge_required_races(seen_factions: &IndexSet<Faction>, starters: &[ReactionSummary], sibling_factions: &IndexSet<Faction>) -> Option<u64> {
    if excluded(Faction::Horde, Faction::Alliance, seen_factions, starters, sibling_factions) {
        return Some(ALLIANCE_RACES);
    }
    if excluded(Faction::Alliance, Faction::Horde, seen_factions, starters, sibling_factions) {
        return Some(HORDE_RACES);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::SessionId;
    use crate::test_support::{event, packed, session_json, stream, with_session};
    use serde_json::{json, Value as Json};
    use Faction::{Alliance, Horde};

    fn faction_group(name: &str) -> Json {
        json!({ "n": 2, "1": name, "2": name })
    }

    fn player_faction(name: &str, t: f64) -> Json {
        json!({ "player": stream([(t, faction_group(name))]) })
    }

    fn observed(session: &Json) -> Vec<(i64, Faction)> {
        with_session(session, |s| observe_quest_factions(s).into_iter().map(|o| (o.entity_id, o.value)).collect())
    }

    /// TS `reactions(faction, friendly, sessions, firstSessionKey = 1)`: one observation in each
    /// of `sessions` sessions, numbered from `first_session`.
    fn reactions(faction: Faction, friendly: bool, sessions: u32, first_session: u32) -> Vec<Observation<FactionReactionValue>> {
        (0..sessions)
            .map(|index| Observation {
                entity_id: 500,
                value: FactionReactionValue { faction, friendly, session_time: None },
                confidence: Confidence::High,
                t: 0.0,
                session: SessionId(first_session + index),
            })
            .collect()
    }

    fn summarize(groups: &[Vec<Observation<FactionReactionValue>>]) -> ReactionSummary {
        summarize_reactions(&groups.concat())
    }

    /// An NPC hostile to `hostile_faction` in `hostile_sessions` sessions and friendly to the
    /// other faction in one.
    fn faction_npc(hostile_faction: Faction, hostile_sessions: u32) -> ReactionSummary {
        let friendly_faction = if hostile_faction == Alliance { Horde } else { Alliance };
        summarize(&[reactions(hostile_faction, false, hostile_sessions, 1), reactions(friendly_faction, true, 1, 1000)])
    }

    fn factions(factions: &[Faction]) -> IndexSet<Faction> {
        factions.iter().copied().collect()
    }

    fn counts(hostile_sessions: usize, friendly_sessions: usize) -> FactionReactionCounts {
        FactionReactionCounts { hostile_sessions, friendly_sessions }
    }

    // describe("observeQuestFactions")

    #[test]
    fn records_the_players_faction_for_an_accepted_quest() {
        let session = session_json(json!({ "UnitFactionGroup": player_faction("Alliance", 0.0) }), vec![event(5.0, "QUEST_ACCEPTED", packed(&[json!(99001)]))]);
        assert_eq!(observed(&session), [(99001, Alliance)]);
    }

    #[test]
    fn records_quests_shown_in_a_quest_frame_and_in_gossip_lists() {
        let session = session_json(
            json!({
                "UnitFactionGroup": player_faction("Horde", 0.0),
                "GetQuestID": stream([(2.0, json!(99001))]),
                "C_GossipInfo.GetAvailableQuests": stream([(3.0, json!([{ "questID": 99002 }]))]),
                "C_GossipInfo.GetActiveQuests": stream([(4.0, json!([{ "questID": 99003 }]))]),
            }),
            vec![],
        );
        assert_eq!(observed(&session), [(99001, Horde), (99002, Horde), (99003, Horde)]);
    }

    #[test]
    fn records_quests_that_are_in_the_quest_log() {
        let session = session_json(
            json!({ "UnitFactionGroup": player_faction("Horde", 0.0), "QuestLog": stream([(0.0, json!([99001, 99002])), (5.0, json!([99002]))]) }),
            vec![],
        );
        assert_eq!(observed(&session), [(99001, Horde), (99002, Horde)]);
    }

    #[test]
    fn uses_the_first_known_faction_for_sightings_before_the_first_faction_sample() {
        let session = session_json(json!({ "UnitFactionGroup": player_faction("Horde", 5.0), "QuestLog": stream([(0.0, json!([99001]))]) }), vec![]);
        assert_eq!(observed(&session), [(99001, Horde)]);
    }

    #[test]
    fn records_each_quest_only_once_per_session() {
        let session = session_json(
            json!({ "UnitFactionGroup": player_faction("Alliance", 0.0), "GetQuestID": stream([(2.0, json!(99001))]) }),
            vec![event(5.0, "QUEST_ACCEPTED", packed(&[json!(99001)]))],
        );
        assert_eq!(observed(&session).len(), 1);
    }

    #[test]
    fn ignores_unfactioned_players() {
        let session = session_json(json!({ "UnitFactionGroup": player_faction("Neutral", 0.0) }), vec![event(5.0, "QUEST_ACCEPTED", packed(&[json!(99001)]))]);
        assert_eq!(observed(&session), []);
    }

    // describe("summarizeReactions")

    #[test]
    fn counts_hostile_and_friendly_sessions_per_faction() {
        let summary = summarize(&[reactions(Horde, false, 3, 1), reactions(Horde, false, 1, 3), reactions(Alliance, true, 2, 1)]);
        assert_eq!(summary, ReactionSummary { alliance: counts(0, 2), horde: counts(3, 0) });
    }

    // describe("mergeRequiredRaces")

    #[test]
    fn excludes_a_faction_when_the_quest_giver_is_hostile_to_it_in_3_sessions() {
        assert_eq!(merge_required_races(&factions(&[Alliance]), &[faction_npc(Horde, 3)], &factions(&[])), Some(ALLIANCE_RACES));
        assert_eq!(merge_required_races(&factions(&[Horde]), &[faction_npc(Alliance, 3)], &factions(&[])), Some(HORDE_RACES));
    }

    #[test]
    fn does_not_restrict_races_when_only_one_faction_was_observed() {
        assert_eq!(merge_required_races(&factions(&[Alliance]), &[], &factions(&[])), None);
    }

    #[test]
    fn does_not_exclude_a_faction_when_the_quest_giver_was_hostile_to_it_in_fewer_than_3_sessions() {
        assert_eq!(merge_required_races(&factions(&[Alliance]), &[faction_npc(Horde, 2)], &factions(&[])), None);
    }

    #[test]
    fn does_not_exclude_a_faction_when_any_of_its_sessions_saw_the_quest_giver_friendly() {
        let summary = summarize(&[reactions(Horde, false, 3, 1), reactions(Horde, true, 1, 100), reactions(Alliance, true, 1, 200)]);
        assert_eq!(merge_required_races(&factions(&[Alliance]), &[summary], &factions(&[])), None);
    }

    #[test]
    fn does_not_exclude_a_faction_when_the_quest_giver_is_unfriendly_to_both_factions() {
        let summary = summarize(&[reactions(Horde, false, 3, 1), reactions(Alliance, false, 1, 100)]);
        assert_eq!(merge_required_races(&factions(&[Alliance]), &[summary], &factions(&[])), None);
    }

    #[test]
    fn does_not_exclude_a_faction_when_the_quest_giver_was_never_seen_friendly_to_the_other_faction() {
        let summary = summarize(&[reactions(Horde, false, 3, 1)]);
        assert_eq!(merge_required_races(&factions(&[Alliance]), &[summary], &factions(&[])), None);
    }

    #[test]
    fn does_not_exclude_a_faction_when_the_other_faction_also_saw_the_quest_giver_hostile() {
        let summary = summarize(&[reactions(Horde, false, 3, 1), reactions(Alliance, true, 5, 100), reactions(Alliance, false, 1, 200)]);
        assert_eq!(merge_required_races(&factions(&[Alliance]), &[summary], &factions(&[])), None);
    }

    #[test]
    fn uses_questies_forever_faction_masks() {
        assert_eq!(ALLIANCE_RACES, 4294967373);
        assert_eq!(HORDE_RACES, 8589934770);
    }

    #[test]
    fn does_not_exclude_a_faction_when_any_of_its_sessions_saw_the_quest() {
        assert_eq!(merge_required_races(&factions(&[Alliance, Horde]), &[faction_npc(Horde, 3)], &factions(&[])), None);
    }

    #[test]
    fn does_not_exclude_a_faction_when_another_quest_giver_might_be_friendly_to_it() {
        let starters = [faction_npc(Horde, 3), summarize_reactions(&[])];
        assert_eq!(merge_required_races(&factions(&[Alliance]), &starters, &factions(&[])), None);
    }

    #[test]
    fn does_not_restrict_races_when_no_session_of_the_remaining_faction_saw_the_quest() {
        assert_eq!(merge_required_races(&factions(&[]), &[faction_npc(Horde, 3)], &factions(&[])), None);
    }

    #[test]
    fn does_not_exclude_a_faction_that_saw_another_quest_of_the_same_quest_giver() {
        assert_eq!(merge_required_races(&factions(&[Alliance]), &[faction_npc(Horde, 3)], &factions(&[Horde])), None);
    }

    #[test]
    fn excludes_a_faction_when_only_the_remaining_faction_saw_other_quests_of_the_same_quest_giver() {
        assert_eq!(merge_required_races(&factions(&[Alliance]), &[faction_npc(Horde, 3)], &factions(&[Alliance])), Some(ALLIANCE_RACES));
    }

    // describe("mergeQuestFactions")

    #[test]
    fn merge_quest_factions_collects_the_factions_of_all_observations() {
        let observations = [Observation::new(1, Alliance, Confidence::High, 0.0), Observation::new(1, Horde, Confidence::High, 1.0)];
        assert_eq!(merge_quest_factions(&observations), factions(&[Alliance, Horde]));
    }
}
