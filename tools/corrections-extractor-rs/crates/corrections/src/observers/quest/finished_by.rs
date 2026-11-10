//! questKeys.finishedBy - Questie field 3. Port of `extract/observers/quest/finishedBy.ts`.
//!
//! `{ creatures, objects }` (no items: quests are not turned in to items) from quest-completion
//! events, each read within its interaction (see [`super::interaction`]):
//!
//! - QUEST_COMPLETE: GetQuestID() for the quest.
//! - QUEST_TURNED_IN: the quest id from the event (argument 1).
//! - GOSSIP_SHOW: the last observed C_GossipInfo.GetActiveQuests() list, replayed against the
//!   current giver (active quests are in the log and can be turned in here).
//!
//! The finisher is UnitGUID("questnpc") when it identifies one; "npc" is only a fallback, not a
//! second finisher.
//!
//! Merge: the union of each finisher kind across all observations.

use super::interaction::{quest_event_lookup, InteractionLookup, InteractionValue};
use crate::guid::{parse_guid, GuidKind};
use crate::observation::{Confidence, Observation};
use std::collections::BTreeSet;
use trace_data::{KeyRef, Session, Value};

/// One finisher seen for a quest (TS `FinishedByFinisher`, which sets exactly one of its fields).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FinishedByFinisher {
    Creature(i64),
    Object(i64),
}

/// The merged finishers, each list ascending.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FinishedByValue {
    pub creatures: Vec<i64>,
    pub objects: Vec<i64>,
}

const GIVER_TOKENS: [&str; 2] = ["questnpc", "npc"];

pub fn merge(observations: &[Observation<FinishedByFinisher>]) -> FinishedByValue {
    let (mut creatures, mut objects) = (BTreeSet::new(), BTreeSet::new());
    for obs in observations {
        // TS skips falsy ids.
        match obs.value {
            FinishedByFinisher::Creature(id) if id != 0 => creatures.insert(id),
            FinishedByFinisher::Object(id) if id != 0 => objects.insert(id),
            _ => false,
        };
    }
    FinishedByValue { creatures: creatures.into_iter().collect(), objects: objects.into_iter().collect() }
}

/// GetQuestID() during the interaction; 0 and an ambiguous sample are no quest.
fn quest_id_at(lookup: &InteractionLookup) -> Option<f64> {
    match lookup.get("GetQuestID", &[]) {
        InteractionValue::Known(value) => value?.as_num().filter(|&id| id != 0.0),
        InteractionValue::Ambiguous => None,
    }
}

/// The quest's finisher on questnpc, else on npc. An ambiguous token ends the search: falling
/// back to npc could pick another interaction's finisher.
fn push_finisher(quest_id: f64, t: f64, lookup: &InteractionLookup, observations: &mut Vec<Observation<FinishedByFinisher>>) {
    for token in GIVER_TOKENS {
        let guid = match lookup.get("UnitGUID", &[KeyRef::Str(token)]) {
            InteractionValue::Ambiguous => return,
            InteractionValue::Known(guid) => guid.and_then(Value::as_str),
        };
        let Some(guid) = guid else { continue };
        let parsed = parse_guid(Some(guid));
        let Some(id) = parsed.id.filter(|&id| id != 0.0) else { continue };
        let (finisher, confidence) = match parsed.kind {
            GuidKind::Npc => (FinishedByFinisher::Creature(id as i64), Confidence::Medium),
            GuidKind::Object => (FinishedByFinisher::Object(id as i64), Confidence::Low),
            _ => continue,
        };
        observations.push(Observation::new(quest_id as i64, finisher, confidence, t));
        return;
    }
}

pub fn observe(session: &Session) -> Vec<Observation<FinishedByFinisher>> {
    let mut observations = Vec::new();

    for (index, event) in session.events().enumerate() {
        if !(event.is("QUEST_COMPLETE") || event.is("QUEST_TURNED_IN") || event.is("GOSSIP_SHOW")) {
            continue;
        }
        let Some(lookup) = quest_event_lookup(session, index) else { continue };

        // Reuse the last observed list at each interaction, not just when its contents change.
        // Missing samples retain last-observed state; failed probes cannot be distinguished.
        if event.is("GOSSIP_SHOW") {
            if let InteractionValue::Known(Some(quests)) = lookup.get("C_GossipInfo.GetActiveQuests", &[]) {
                for quest in quests.as_array().unwrap_or(&[]) {
                    if let Some(quest_id) = quest.get("questID").and_then(Value::as_num).filter(|&id| id > 0.0) {
                        push_finisher(quest_id, event.t(), &lookup, &mut observations);
                    }
                }
            }
            continue;
        }

        let quest_id = if event.is("QUEST_COMPLETE") {
            quest_id_at(&lookup)
        } else if event.arg_count().is_some_and(|n| n >= 1.0) {
            // QUEST_TURNED_IN
            event.arg(1).and_then(Value::as_num).filter(|&id| id != 0.0)
        } else {
            None
        };
        let Some(quest_id) = quest_id else { continue };

        push_finisher(quest_id, event.t(), &lookup, &mut observations);
    }

    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observers::quest::interaction::fixtures::{creature_guid, object_guid, sample, trace_event};
    use crate::test_support::{session_json, with_session};
    use serde_json::{json, Value as Json};
    use FinishedByFinisher::{Creature, Object};

    fn observed(session: &Json) -> Vec<(i64, FinishedByFinisher, Confidence, f64)> {
        with_session(session, |s| observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect())
    }

    fn values(session: &Json) -> Vec<FinishedByFinisher> {
        observed(session).into_iter().map(|(_, value, _, _)| value).collect()
    }

    fn quests_and_values(session: &Json) -> Vec<(i64, FinishedByFinisher)> {
        observed(session).into_iter().map(|(quest, value, _, _)| (quest, value)).collect()
    }

    fn obs(value: FinishedByFinisher, confidence: Confidence, t: f64) -> Observation<FinishedByFinisher> {
        Observation::new(96659, value, confidence, t)
    }

    const GAME_OBJECT_2843: &str = "GameObject-0-5208-0-7-2843-0000399";

    #[test]
    fn captures_a_creature_finisher_from_the_questnpc_guid_at_quest_complete() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.0, &[])],
        );
        assert_eq!(observed(&session), [(96659, Creature(197), Confidence::Medium, 3.0)]);
    }

    #[test]
    fn captures_a_creature_finisher_from_the_npc_guid_when_questnpc_is_absent() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "npc": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.0, &[])],
        );
        assert_eq!(observed(&session), [(96659, Creature(197), Confidence::Medium, 3.0)]);
    }

    #[test]
    fn captures_an_object_finisher_from_a_game_object_guid_on_the_npc_token() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "npc": [sample(3.0, 3.0, json!(GAME_OBJECT_2843))] } }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.0, &[])],
        );
        assert_eq!(observed(&session), [(96659, Object(2843), Confidence::Low, 3.0)]);
    }

    #[test]
    fn prefers_the_questnpc_creature_over_a_conflicting_npc_object() {
        let session = session_json(
            json!({
                "GetQuestID": [sample(3.0, 3.0, json!(96659))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))], "npc": [sample(3.0, 3.0, json!(GAME_OBJECT_2843))] },
            }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.0, &[])],
        );
        assert_eq!(values(&session), [Creature(197)]);
    }

    #[test]
    fn prefers_questnpc_over_npc_when_both_are_present() {
        let session = session_json(
            json!({
                "GetQuestID": [sample(3.0, 3.0, json!(96659))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))], "npc": [sample(3.0, 3.0, json!("Creature-0-5208-0-7-200-000040"))] },
            }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.0, &[])],
        );
        let results = with_session(&session, observe);
        assert_eq!(merge(&results), FinishedByValue { creatures: vec![197], objects: vec![] });
    }

    #[test]
    fn captures_creature_finishers_across_different_quest_events() {
        let session = session_json(
            json!({
                "GetQuestID": [sample(3.0, 3.0, json!(96659)), sample(10.0, 10.0, json!(12345))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197)), sample(10.0, 10.0, json!("Creature-0-5208-0-7-200-000040"))] },
            }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.0, &[]), trace_event("QUEST_COMPLETE", 10.0, 10.0, &[])],
        );
        assert_eq!(quests_and_values(&session), [(96659, Creature(197)), (12345, Creature(200))]);
    }

    #[test]
    fn ignores_player_guids_on_quest_tokens() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, json!("Player-5284-0362"))] } }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.0, &[])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn observes_nothing_without_quest_completion_events() {
        let session =
            session_json(json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }), vec![]);
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn observes_nothing_when_get_quest_id_is_not_active_at_the_event() {
        let session =
            session_json(json!({ "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }), vec![trace_event("QUEST_COMPLETE", 3.0, 3.0, &[])]);
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn captures_a_creature_finisher_from_quest_turned_in_argument_1() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(276171))] } }),
            vec![trace_event("QUEST_TURNED_IN", 3.0, 3.0, &[json!(99196)])],
        );
        assert_eq!(observed(&session), [(99196, Creature(276171), Confidence::Medium, 3.0)]);
    }

    #[test]
    fn captures_an_object_finisher_from_quest_turned_in() {
        let session = session_json(
            json!({ "UnitGUID": { "npc": [sample(3.0, 3.0, json!(GAME_OBJECT_2843))] } }),
            vec![trace_event("QUEST_TURNED_IN", 3.0, 3.0, &[json!(99196)])],
        );
        assert_eq!(observed(&session), [(99196, Object(2843), Confidence::Low, 3.0)]);
    }

    #[test]
    fn ignores_quest_turned_in_with_an_invalid_quest_id() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(276171))] } }),
            vec![
                trace_event("QUEST_TURNED_IN", 3.0, 3.0, &[json!(0)]),
                trace_event("QUEST_TURNED_IN", 4.0, 4.0, &[json!("99196")]),
                trace_event("QUEST_TURNED_IN", 5.0, 5.0, &[]),
            ],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn observes_both_quest_complete_and_quest_turned_in_for_the_same_quest() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.0, &[]), trace_event("QUEST_TURNED_IN", 4.0, 4.0, &[json!(96659)])],
        );
        let results = with_session(&session, observe);
        // One from each event; the merge keeps the creature once.
        assert_eq!(results.iter().filter(|o| matches!(o.value, Creature(_))).count(), 2);
        assert_eq!(merge(&results).creatures, [197]);
    }

    #[test]
    fn captures_a_creature_finisher_from_the_gossip_active_quests() {
        let session = session_json(
            json!({
                "C_GossipInfo.GetActiveQuests": [sample(3.0, 3.0, json!([{ "questID": 99196, "title": "A Donation of Wool", "repeatable": false, "isLegendary": false }]))],
                "UnitGUID": { "npc": [sample(3.0, 3.0, creature_guid(276171))] },
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.0, &[])],
        );
        assert_eq!(observed(&session), [(99196, Creature(276171), Confidence::Medium, 3.0)]);
    }

    #[test]
    fn captures_every_quest_of_one_gossip_active_quests_list() {
        let session = session_json(
            json!({
                "C_GossipInfo.GetActiveQuests": [sample(3.0, 3.0, json!([
                    { "questID": 99196, "title": "A Donation of Wool", "repeatable": false },
                    { "questID": 99197, "title": "Another Quest", "repeatable": false },
                ]))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(276171))] },
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.0, &[])],
        );
        let results = quests_and_values(&session);
        let mut quests: Vec<i64> = results.iter().map(|&(q, _)| q).collect();
        quests.sort();
        assert_eq!(quests, [99196, 99197]);
        assert_eq!(results.iter().map(|&(_, v)| v).collect::<Vec<_>>(), [Creature(276171), Creature(276171)]);
    }

    #[test]
    fn ignores_gossip_active_quests_without_a_valid_quest_id() {
        let session = session_json(
            json!({
                "C_GossipInfo.GetActiveQuests": [sample(3.0, 3.0, json!([
                    { "title": "No ID here", "repeatable": false },
                    { "questID": 0, "title": "Zero ID", "repeatable": false },
                ]))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(276171))] },
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.0, &[])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn merge_keeps_each_creature_finisher_once() {
        let result =
            merge(&[obs(Creature(197), Confidence::Medium, 3.0), obs(Creature(197), Confidence::Medium, 10.0), obs(Creature(200), Confidence::Medium, 15.0)]);
        assert_eq!(result, FinishedByValue { creatures: vec![197, 200], objects: vec![] });
    }

    #[test]
    fn merge_sorts_finishers_by_kind() {
        let result = merge(&[obs(Creature(197), Confidence::Medium, 3.0), obs(Object(2843), Confidence::Low, 5.0)]);
        assert_eq!(result, FinishedByValue { creatures: vec![197], objects: vec![2843] });
    }

    #[test]
    fn attributes_a_guid_sampled_long_before_the_quest_event() {
        // questnpc/npc are only recorded on change (UnitInteraction.lua), so reading quest text
        // for a long time leaves the last GUID sample far behind the event; it is not stale.
        let session = session_json(
            json!({ "GetQuestID": [sample(60.0, 60.0, json!(96659))], "UnitGUID": { "questnpc": [sample(1.0, 1.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_COMPLETE", 60.0, 60.0, &[])],
        );
        assert_eq!(observed(&session), [(96659, Creature(197), Confidence::Medium, 60.0)]);
    }

    // describe("observeFinishedBy interaction correlation")

    #[test]
    fn prefers_a_questnpc_object_over_a_conflicting_npc_creature() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(3.0, 3.002, object_guid(410369))], "npc": [sample(3.0, 3.002, creature_guid(276172))] } }),
            vec![trace_event("QUEST_TURNED_IN", 3.0, 3.001, &[json!(99196)])],
        );
        assert_eq!(values(&session), [Object(410369)]);
    }

    #[test]
    fn falls_back_to_npc_when_questnpc_is_nil() {
        let session = session_json(
            json!({ "UnitGUID": {
                "questnpc": [sample(1.0, 1.0, creature_guid(276171)), sample(3.0, 3.002, None)],
                "npc": [sample(3.0, 3.002, creature_guid(276172))],
            } }),
            vec![trace_event("QUEST_TURNED_IN", 3.0, 3.001, &[json!(99196)])],
        );
        assert_eq!(values(&session), [Creature(276172)]);
    }

    #[test]
    fn includes_synchronous_probes_without_borrowing_a_later_same_frame_quest_or_giver() {
        let session = session_json(
            json!({
                "GetQuestID": [sample(3.0, 3.002, json!(99196)), sample(3.0, 3.012, json!(99197))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.003, creature_guid(276171)), sample(3.0, 3.013, creature_guid(276172))] },
            }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.001, &[]), trace_event("QUEST_COMPLETE", 3.0, 3.011, &[])],
        );
        assert_eq!(quests_and_values(&session), [(99196, Creature(276171)), (99197, Creature(276172))]);
    }

    #[test]
    fn uses_the_next_raw_event_as_the_boundary_even_when_it_is_not_a_quest_event() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(1.0, 1.0, creature_guid(276171)), sample(3.0, 3.012, creature_guid(276172))] } }),
            vec![trace_event("QUEST_TURNED_IN", 3.0, 3.001, &[json!(99196)]), trace_event("MERCHANT_SHOW", 3.0, 3.011, &[])],
        );
        assert_eq!(values(&session), [Creature(276171)]);
    }

    #[test]
    fn does_not_let_a_later_same_frame_close_erase_the_giver_or_reuse_it_after_close() {
        let session = session_json(
            json!({ "UnitGUID": { "npc": [sample(1.0, 1.0, object_guid(410369)), sample(3.0, 3.012, None)] } }),
            vec![
                trace_event("QUEST_TURNED_IN", 3.0, 3.001, &[json!(99196)]),
                trace_event("QUEST_FINISHED", 3.0, 3.011, &[]),
                trace_event("QUEST_TURNED_IN", 3.0, 3.021, &[json!(99197)]),
            ],
        );
        assert_eq!(quests_and_values(&session), [(99196, Object(410369))]);
    }

    #[test]
    fn keeps_an_unchanged_object_giver_during_a_long_dialog() {
        let session = session_json(
            json!({ "UnitGUID": { "npc": [sample(1.0, 1.0, object_guid(410369))] } }),
            vec![trace_event("QUEST_PROGRESS", 1.0, 1.0, &[]), trace_event("QUEST_TURNED_IN", 60.0, 60.0, &[json!(99196)])],
        );
        assert_eq!(values(&session), [Object(410369)]);
    }

    #[test]
    fn skips_events_with_indistinguishable_timestamps_instead_of_guessing_their_giver() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(276171)), sample(3.0, 3.0, creature_guid(276172))] } }),
            vec![trace_event("QUEST_TURNED_IN", 3.0, 3.0, &[json!(99196)]), trace_event("QUEST_TURNED_IN", 3.0, 3.0, &[json!(99197)])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn does_not_resurrect_an_older_giver_when_a_probe_ties_with_the_next_event() {
        for tied_guid in [None, Some(creature_guid(276172))] {
            let session = session_json(
                json!({ "UnitGUID": {
                    "questnpc": [sample(1.0, 1.0, creature_guid(276171)), sample(3.0, 3.002, tied_guid.clone())],
                    "npc": [sample(1.0, 1.0, creature_guid(276173))],
                } }),
                vec![trace_event("QUEST_TURNED_IN", 3.0, 3.001, &[json!(99196)]), trace_event("MERCHANT_SHOW", 3.0, 3.002, &[])],
            );
            assert_eq!(observed(&session), [], "tied guid {tied_guid:?}");
        }
    }

    #[test]
    fn does_not_resurrect_an_older_quest_id_when_its_probe_ties_with_the_next_event() {
        let session = session_json(
            json!({
                "GetQuestID": [sample(1.0, 1.0, json!(99196)), sample(3.0, 3.002, json!(99197))],
                "UnitGUID": { "questnpc": [sample(1.0, 1.0, creature_guid(276171))] },
            }),
            vec![trace_event("QUEST_COMPLETE", 3.0, 3.001, &[]), trace_event("QUEST_FINISHED", 3.0, 3.002, &[])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn skips_gossip_events_with_indistinguishable_timestamps() {
        let session = session_json(
            json!({
                "UnitGUID": { "npc": [sample(1.0, 1.0, creature_guid(276171)), sample(3.0, 3.002, creature_guid(276172))] },
                "C_GossipInfo.GetActiveQuests": [sample(1.0, 1.0, json!([{ "questID": 99196 }]))],
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.001, &[]), trace_event("GOSSIP_SHOW", 3.0, 3.001, &[])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn does_not_infer_an_interaction_from_a_list_without_a_gossip_show_event() {
        let session = session_json(
            json!({
                "UnitGUID": { "npc": [sample(3.0, 3.002, creature_guid(276171))] },
                "C_GossipInfo.GetActiveQuests": [sample(3.0, 3.002, json!([{ "questID": 99196 }]))],
            }),
            vec![],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn replays_one_unchanged_gossip_list_at_two_npcs_within_the_same_frame() {
        let session = session_json(
            json!({
                "UnitGUID": { "npc": [sample(3.0, 3.002, creature_guid(276171)), sample(3.0, 3.012, creature_guid(276172))] },
                "C_GossipInfo.GetActiveQuests": [sample(3.0, 3.002, json!([{ "questID": 99196 }]))],
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.001, &[]), trace_event("GOSSIP_SHOW", 3.0, 3.011, &[])],
        );
        assert_eq!(quests_and_values(&session), [(99196, Creature(276171)), (99196, Creature(276172))]);
    }

    #[test]
    fn replays_the_last_observed_list_after_closing_and_opening_gossip_at_another_npc() {
        let session = session_json(
            json!({
                "UnitGUID": { "npc": [sample(1.0, 1.002, creature_guid(276171)), sample(2.0, 2.002, None), sample(3.0, 3.002, creature_guid(276172))] },
                "C_GossipInfo.GetActiveQuests": [sample(1.0, 1.002, json!([{ "questID": 99196 }]))],
            }),
            vec![trace_event("GOSSIP_SHOW", 1.0, 1.001, &[]), trace_event("GOSSIP_CLOSED", 2.0, 2.001, &[]), trace_event("GOSSIP_SHOW", 3.0, 3.001, &[])],
        );
        assert_eq!(values(&session), [Creature(276171), Creature(276172)]);
    }

    #[test]
    fn does_not_carry_quests_past_an_observed_empty_or_nil_list() {
        for clearing_list in [Some(json!([])), None] {
            let session = session_json(
                json!({
                    "UnitGUID": { "npc": [sample(1.0, 1.002, creature_guid(276171)), sample(3.0, 3.002, creature_guid(276172))] },
                    "C_GossipInfo.GetActiveQuests": [sample(1.0, 1.002, json!([{ "questID": 99196 }])), sample(3.0, 3.002, clearing_list.clone())],
                }),
                vec![trace_event("GOSSIP_SHOW", 1.0, 1.001, &[]), trace_event("GOSSIP_SHOW", 3.0, 3.001, &[]), trace_event("GOSSIP_SHOW", 4.0, 4.001, &[])],
            );
            assert_eq!(values(&session), [Creature(276171)], "clearing list {clearing_list:?}");
        }
    }

    #[test]
    fn does_not_read_a_later_same_frame_quest_list_into_an_earlier_gossip_interaction() {
        let session = session_json(
            json!({
                "UnitGUID": { "npc": [sample(3.0, 3.002, creature_guid(276171)), sample(3.0, 3.012, creature_guid(276172))] },
                "C_GossipInfo.GetActiveQuests": [sample(3.0, 3.002, json!([{ "questID": 99196 }])), sample(3.0, 3.012, json!([{ "questID": 99197 }]))],
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.001, &[]), trace_event("GOSSIP_SHOW", 3.0, 3.011, &[])],
        );
        assert_eq!(quests_and_values(&session), [(99196, Creature(276171)), (99197, Creature(276172))]);
    }
}
