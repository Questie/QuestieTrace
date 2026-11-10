//! questKeys.startedBy - Questie field 2. Port of `extract/observers/quest/startedBy.ts`.
//!
//! `{ creatures, objects, items }` from quest-start events, each read within its interaction
//! (see [`super::interaction`]):
//!
//! - QUEST_DETAIL: GetQuestID() for the quest, and its questStartItemID argument for an item
//!   starter.
//! - QUEST_ACCEPTED: the quest id from the event.
//! - GOSSIP_SHOW: the last observed C_GossipInfo.GetAvailableQuests() list, replayed against
//!   the current giver (available quests can be started here).
//!
//! The giver is UnitGUID("questnpc") when it identifies one. "npc" is only a fallback for
//! gossip, where questnpc is never set: at QUEST_DETAIL/QUEST_ACCEPTED it is whatever the
//! player is using (a mailbox or vendor while a party member shares a quest). Object starters
//! always show up on questnpc, so npc never yields an object. "target" is never used: it can
//! point at anything and is not tied to the interaction.
//!
//! Merge: the union of each starter kind across all observations.

use super::interaction::{quest_event_lookup, InteractionLookup, InteractionValue};
use crate::guid::{parse_guid, GuidKind};
use crate::observation::{Confidence, Observation};
use std::collections::BTreeSet;
use trace_data::{KeyRef, Session, Value};

/// One starter seen for a quest (TS `StartedByStarter`, which sets exactly one of its fields).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StartedByStarter {
    Creature(i64),
    Object(i64),
    Item(i64),
}

/// The merged starters, each list ascending.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StartedByValue {
    pub creatures: Vec<i64>,
    pub objects: Vec<i64>,
    pub items: Vec<i64>,
}

const GOSSIP_TOKENS: [&str; 2] = ["questnpc", "npc"];
const QUEST_DIALOG_TOKENS: [&str; 1] = ["questnpc"];

pub fn merge(observations: &[Observation<StartedByStarter>]) -> StartedByValue {
    let (mut creatures, mut objects, mut items) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for obs in observations {
        // TS skips falsy ids.
        match obs.value {
            StartedByStarter::Creature(id) if id != 0 => creatures.insert(id),
            StartedByStarter::Object(id) if id != 0 => objects.insert(id),
            StartedByStarter::Item(id) if id != 0 => items.insert(id),
            _ => false,
        };
    }
    StartedByValue { creatures: creatures.into_iter().collect(), objects: objects.into_iter().collect(), items: items.into_iter().collect() }
}

/// GetQuestID() during the interaction; 0 and an ambiguous sample are no quest.
fn quest_id_at(lookup: &InteractionLookup) -> Option<f64> {
    match lookup.get("GetQuestID", &[]) {
        InteractionValue::Known(value) => value?.as_num().filter(|&id| id != 0.0),
        InteractionValue::Ambiguous => None,
    }
}

/// The quest's giver on the first of `tokens` that identifies one. An ambiguous token ends the
/// search: falling back to the next token could pick another interaction's giver.
fn push_giver(quest_id: f64, t: f64, lookup: &InteractionLookup, tokens: &[&str], observations: &mut Vec<Observation<StartedByStarter>>) {
    for &token in tokens {
        let guid = match lookup.get("UnitGUID", &[KeyRef::Str(token)]) {
            InteractionValue::Ambiguous => return,
            InteractionValue::Known(guid) => guid.and_then(Value::as_str),
        };
        let Some(guid) = guid else { continue };
        let parsed = parse_guid(Some(guid));
        let Some(id) = parsed.id.filter(|&id| id != 0.0) else { continue };
        match parsed.kind {
            GuidKind::Npc => {
                observations.push(Observation::new(quest_id as i64, StartedByStarter::Creature(id as i64), Confidence::Medium, t));
                return;
            }
            GuidKind::Object if token == "questnpc" => {
                observations.push(Observation::new(quest_id as i64, StartedByStarter::Object(id as i64), Confidence::Low, t));
                return;
            }
            _ => {}
        }
    }
}

pub fn observe(session: &Session) -> Vec<Observation<StartedByStarter>> {
    let mut observations = Vec::new();

    for (index, event) in session.events().enumerate() {
        if !(event.is("QUEST_DETAIL") || event.is("QUEST_ACCEPTED") || event.is("GOSSIP_SHOW")) {
            continue;
        }
        let Some(lookup) = quest_event_lookup(session, index) else { continue };

        // Lists are change-only: each GOSSIP_SHOW can expose the same quests at a different NPC.
        // As with GUID replay, a failed probe is indistinguishable from an unchanged value.
        if event.is("GOSSIP_SHOW") {
            if let InteractionValue::Known(Some(quests)) = lookup.get("C_GossipInfo.GetAvailableQuests", &[]) {
                for quest in quests.as_array().unwrap_or(&[]) {
                    if let Some(quest_id) = quest.get("questID").and_then(Value::as_num).filter(|&id| id > 0.0) {
                        push_giver(quest_id, event.t(), &lookup, &GOSSIP_TOKENS, &mut observations);
                    }
                }
            }
            continue;
        }

        let mut starter_item_id = None;
        let quest_id = if event.is("QUEST_DETAIL") {
            if event.arg_count().is_some_and(|n| n >= 1.0) {
                starter_item_id = event.arg(1).and_then(Value::as_num);
            }
            quest_id_at(&lookup)
        } else {
            // QUEST_ACCEPTED: Era, TBC, Wrath and Mists send (questLogIndex, questID), Forever (questID).
            let args = event.args();
            let quest_id = if args.and_then(|a| a.get("n")).and_then(Value::as_num) == Some(1.0) { event.arg(1) } else { event.arg(2) };
            quest_id.and_then(Value::as_num).filter(|&id| id != 0.0)
        };
        let Some(quest_id) = quest_id else { continue };

        push_giver(quest_id, event.t(), &lookup, &QUEST_DIALOG_TOKENS, &mut observations);

        if let Some(item_id) = starter_item_id.filter(|&id| id > 0.0) {
            observations.push(Observation::new(quest_id as i64, StartedByStarter::Item(item_id as i64), Confidence::High, event.t()));
        }
    }

    observations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observers::quest::interaction::fixtures::{creature_guid, object_guid, sample, trace_event};
    use crate::test_support::{session_json, with_session};
    use serde_json::{json, Value as Json};
    use StartedByStarter::{Creature, Item, Object};

    fn observed(session: &Json) -> Vec<(i64, StartedByStarter, Confidence, f64)> {
        with_session(session, |s| observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect())
    }

    fn values(session: &Json) -> Vec<StartedByStarter> {
        observed(session).into_iter().map(|(_, value, _, _)| value).collect()
    }

    fn quests_and_values(session: &Json) -> Vec<(i64, StartedByStarter)> {
        observed(session).into_iter().map(|(quest, value, _, _)| (quest, value)).collect()
    }

    fn obs(value: StartedByStarter, confidence: Confidence, t: f64) -> Observation<StartedByStarter> {
        Observation::new(96659, value, confidence, t)
    }

    #[test]
    fn captures_a_creature_starter_from_the_questnpc_guid_at_quest_detail() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(0)])],
        );
        assert_eq!(observed(&session), [(96659, Creature(197), Confidence::Medium, 3.0)]);
    }

    #[test]
    fn does_not_take_a_creature_starter_from_the_npc_guid_at_quest_detail_without_questnpc() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "npc": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(0)])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn captures_an_item_starter_from_the_quest_detail_quest_start_item_id() {
        let session = session_json(json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))] }), vec![trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(750)])]);
        assert_eq!(observed(&session), [(96659, Item(750), Confidence::High, 3.0)]);
    }

    #[test]
    fn captures_both_a_creature_and_an_item_starter_for_the_same_quest() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(750)])],
        );
        let found = values(&session);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&Creature(197)));
        assert!(found.contains(&Item(750)));
    }

    #[test]
    fn reads_the_quest_id_from_the_one_argument_quest_accepted() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_ACCEPTED", 3.0, 3.0, &[json!(96659)])],
        );
        assert_eq!(observed(&session), [(96659, Creature(197), Confidence::Medium, 3.0)]);
    }

    #[test]
    fn reads_the_quest_id_from_argument_2_of_the_two_argument_quest_accepted() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(11111))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_ACCEPTED", 3.0, 3.0, &[json!(11111), json!(96659)])],
        );
        assert_eq!(observed(&session), [(96659, Creature(197), Confidence::Medium, 3.0)]);
    }

    #[test]
    fn ignores_invalid_quest_accepted_payloads() {
        for args in [json!({ "1": 0, "n": 1 }), json!({ "1": "96659", "n": 1 }), json!({ "n": 0 })] {
            let session = session_json(
                json!({ "GetQuestID": [sample(3.0, 3.0, json!(11111))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }),
                vec![json!({ "t": 3, "tp": 3, "e": "QUEST_ACCEPTED", "a": args })],
            );
            assert_eq!(observed(&session), [], "payload {args}");
        }
    }

    #[test]
    fn captures_creature_starters_across_different_quest_events() {
        let session = session_json(
            json!({
                "GetQuestID": [sample(3.0, 3.0, json!(96659)), sample(10.0, 10.0, json!(12345))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197)), sample(10.0, 10.0, json!("Creature-0-5208-0-7-200-000040"))] },
            }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(12345)]), trace_event("QUEST_DETAIL", 10.0, 10.0, &[json!(67890)])],
        );
        let results = quests_and_values(&session);
        // Each quest event gives a creature and an item observation.
        assert_eq!(results.len(), 4);
        let creatures: Vec<(i64, i64)> = results.iter().filter_map(|&(q, v)| if let Creature(id) = v { Some((q, id)) } else { None }).collect();
        assert_eq!(creatures, [(96659, 197), (12345, 200)]);
        let items: Vec<i64> = results.iter().filter_map(|&(_, v)| if let Item(id) = v { Some(id) } else { None }).collect();
        assert_eq!(items, [12345, 67890]);
    }

    #[test]
    fn ignores_the_target_guid() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "target": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(0)])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn ignores_a_game_object_guid_on_the_target_token() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "target": [sample(3.0, 3.0, json!("GameObject-0-5208-0-7-2843-0000399"))] } }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(0)])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn attributes_a_guid_sampled_long_before_the_quest_event() {
        // questnpc/npc are only recorded on change (UnitInteraction.lua), so reading quest text
        // for a long time leaves the last GUID sample far behind the event; it is not stale.
        let session = session_json(
            json!({ "GetQuestID": [sample(60.0, 60.0, json!(96659))], "UnitGUID": { "questnpc": [sample(1.0, 1.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_DETAIL", 60.0, 60.0, &[json!(0)])],
        );
        assert_eq!(observed(&session), [(96659, Creature(197), Confidence::Medium, 60.0)]);
    }

    #[test]
    fn ignores_player_guids_on_quest_tokens() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, json!("Player-5284-0362"))] } }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(0)])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn observes_nothing_without_quest_start_events() {
        let session =
            session_json(json!({ "GetQuestID": [sample(3.0, 3.0, json!(96659))], "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }), vec![]);
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn observes_nothing_when_get_quest_id_is_not_active_at_the_event() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] } }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(12345)])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn captures_a_creature_starter_from_the_gossip_available_quests() {
        let session = session_json(
            json!({
                "C_GossipInfo.GetAvailableQuests": [sample(3.0, 3.0, json!([{ "questID": 99196, "title": "A Donation of Wool", "repeatable": false, "isLegendary": false }]))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(276171))] },
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.0, &[])],
        );
        assert_eq!(observed(&session), [(99196, Creature(276171), Confidence::Medium, 3.0)]);
    }

    #[test]
    fn captures_every_quest_of_one_gossip_available_quests_list() {
        let session = session_json(
            json!({
                "C_GossipInfo.GetAvailableQuests": [sample(3.0, 3.0, json!([
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
    fn ignores_gossip_quests_without_a_valid_quest_id() {
        let session = session_json(
            json!({
                "C_GossipInfo.GetAvailableQuests": [sample(3.0, 3.0, json!([
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
    fn observes_both_quest_detail_and_gossip_for_the_same_quest() {
        let session = session_json(
            json!({
                "GetQuestID": [sample(3.0, 3.0, json!(96659))],
                "C_GossipInfo.GetAvailableQuests": [sample(3.0, 3.0, json!([{ "questID": 96659, "title": "From Gossip", "repeatable": false }]))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(197))] },
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.0, &[]), trace_event("QUEST_DETAIL", 4.0, 4.0, &[json!(0)])],
        );
        let results = with_session(&session, observe);
        // One from QUEST_DETAIL, one from gossip; the merge keeps the creature once.
        assert_eq!(results.iter().filter(|o| matches!(o.value, Creature(_))).count(), 2);
        assert_eq!(merge(&results).creatures, [197]);
    }

    #[test]
    fn merge_keeps_each_creature_starter_once() {
        let result =
            merge(&[obs(Creature(197), Confidence::Medium, 3.0), obs(Creature(197), Confidence::Medium, 10.0), obs(Creature(200), Confidence::Medium, 15.0)]);
        assert_eq!(result, StartedByValue { creatures: vec![197, 200], objects: vec![], items: vec![] });
    }

    #[test]
    fn merge_sorts_starters_by_kind() {
        let result = merge(&[obs(Creature(197), Confidence::Medium, 3.0), obs(Object(2843), Confidence::Low, 5.0), obs(Item(750), Confidence::High, 7.0)]);
        assert_eq!(result, StartedByValue { creatures: vec![197], objects: vec![2843], items: vec![750] });
    }

    // describe("observeStartedBy interaction correlation")

    #[test]
    fn prefers_a_questnpc_object_over_a_conflicting_npc_creature() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(3.0, 3.002, object_guid(410369))], "npc": [sample(3.0, 3.002, creature_guid(276172))] } }),
            vec![trace_event("QUEST_ACCEPTED", 3.0, 3.001, &[json!(99196)])],
        );
        assert_eq!(values(&session), [Object(410369)]);
    }

    #[test]
    fn does_not_fall_back_to_an_npc_creature_at_quest_accepted_when_questnpc_is_nil() {
        let session = session_json(
            json!({ "UnitGUID": {
                "questnpc": [sample(1.0, 1.0, creature_guid(276171)), sample(3.0, 3.002, None)],
                "npc": [sample(3.0, 3.002, creature_guid(276172))],
            } }),
            vec![trace_event("QUEST_ACCEPTED", 3.0, 3.001, &[json!(99196)])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn falls_back_to_an_npc_creature_for_gossip_available_quests() {
        let session = session_json(
            json!({
                "C_GossipInfo.GetAvailableQuests": [sample(3.0, 3.001, json!([{ "questID": 5042 }]))],
                "UnitGUID": { "npc": [sample(3.0, 3.001, creature_guid(3430))] },
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.0, &[])],
        );
        assert_eq!(quests_and_values(&session), [(5042, Creature(3430))]);
    }

    #[test]
    fn does_not_fall_back_to_an_npc_object_like_a_mailbox_when_a_quest_is_shared() {
        let session = session_json(
            json!({ "GetQuestID": [sample(3.0, 3.0, json!(1486))], "UnitGUID": { "npc": [sample(1.0, 1.0, object_guid(659159))] } }),
            vec![
                trace_event("MAIL_SHOW", 1.0, 1.0, &[]),
                trace_event("QUEST_DETAIL", 3.0, 3.0, &[json!(0)]),
                trace_event("QUEST_ACCEPTED", 4.0, 4.0, &[json!(1486)]),
            ],
        );
        assert_eq!(observed(&session), []);
    }

    // Not in startedBy.test.ts: npc is only read at GOSSIP_SHOW, and none of the TS tests puts
    // an object there, so nothing caught accepting an object on npc.
    #[test]
    fn does_not_take_an_object_starter_from_the_npc_token_at_gossip() {
        let session = session_json(
            json!({
                "C_GossipInfo.GetAvailableQuests": [sample(3.0, 3.002, json!([{ "questID": 5042 }]))],
                "UnitGUID": { "npc": [sample(3.0, 3.002, object_guid(659159))] },
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.001, &[])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn includes_synchronous_probes_without_borrowing_a_later_same_frame_quest_or_giver() {
        let session = session_json(
            json!({
                "GetQuestID": [sample(3.0, 3.002, json!(99196)), sample(3.0, 3.012, json!(99197))],
                "UnitGUID": { "questnpc": [sample(3.0, 3.003, creature_guid(276171)), sample(3.0, 3.013, creature_guid(276172))] },
            }),
            vec![trace_event("QUEST_DETAIL", 3.0, 3.001, &[]), trace_event("QUEST_DETAIL", 3.0, 3.011, &[])],
        );
        assert_eq!(quests_and_values(&session), [(99196, Creature(276171)), (99197, Creature(276172))]);
    }

    #[test]
    fn uses_the_next_raw_event_as_the_boundary_even_when_it_is_not_a_quest_event() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(1.0, 1.0, creature_guid(276171)), sample(3.0, 3.012, creature_guid(276172))] } }),
            vec![trace_event("QUEST_ACCEPTED", 3.0, 3.001, &[json!(99196)]), trace_event("MERCHANT_SHOW", 3.0, 3.011, &[])],
        );
        assert_eq!(values(&session), [Creature(276171)]);
    }

    #[test]
    fn does_not_let_a_later_same_frame_close_erase_the_giver_or_reuse_it_after_close() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(1.0, 1.0, object_guid(410369)), sample(3.0, 3.012, None)] } }),
            vec![
                trace_event("QUEST_ACCEPTED", 3.0, 3.001, &[json!(99196)]),
                trace_event("QUEST_FINISHED", 3.0, 3.011, &[]),
                trace_event("QUEST_ACCEPTED", 3.0, 3.021, &[json!(99197)]),
            ],
        );
        assert_eq!(quests_and_values(&session), [(99196, Object(410369))]);
    }

    #[test]
    fn keeps_an_unchanged_object_giver_during_a_long_dialog() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(1.0, 1.0, object_guid(410369))] } }),
            vec![trace_event("QUEST_PROGRESS", 1.0, 1.0, &[]), trace_event("QUEST_ACCEPTED", 60.0, 60.0, &[json!(99196)])],
        );
        assert_eq!(values(&session), [Object(410369)]);
    }

    #[test]
    fn skips_events_with_indistinguishable_timestamps_instead_of_guessing_their_giver() {
        let session = session_json(
            json!({ "UnitGUID": { "questnpc": [sample(3.0, 3.0, creature_guid(276171)), sample(3.0, 3.0, creature_guid(276172))] } }),
            vec![trace_event("QUEST_ACCEPTED", 3.0, 3.0, &[json!(99196)]), trace_event("QUEST_ACCEPTED", 3.0, 3.0, &[json!(99197)])],
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
                vec![trace_event("QUEST_ACCEPTED", 3.0, 3.001, &[json!(99196)]), trace_event("MERCHANT_SHOW", 3.0, 3.002, &[])],
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
            vec![trace_event("QUEST_DETAIL", 3.0, 3.001, &[]), trace_event("QUEST_FINISHED", 3.0, 3.002, &[])],
        );
        assert_eq!(observed(&session), []);
    }

    #[test]
    fn skips_gossip_events_with_indistinguishable_timestamps() {
        let session = session_json(
            json!({
                "UnitGUID": { "npc": [sample(1.0, 1.0, creature_guid(276171)), sample(3.0, 3.002, creature_guid(276172))] },
                "C_GossipInfo.GetAvailableQuests": [sample(1.0, 1.0, json!([{ "questID": 99196 }]))],
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
                "C_GossipInfo.GetAvailableQuests": [sample(3.0, 3.002, json!([{ "questID": 99196 }]))],
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
                "C_GossipInfo.GetAvailableQuests": [sample(3.0, 3.002, json!([{ "questID": 99196 }]))],
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
                "C_GossipInfo.GetAvailableQuests": [sample(1.0, 1.002, json!([{ "questID": 99196 }]))],
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
                    "C_GossipInfo.GetAvailableQuests": [sample(1.0, 1.002, json!([{ "questID": 99196 }])), sample(3.0, 3.002, clearing_list.clone())],
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
                "C_GossipInfo.GetAvailableQuests": [sample(3.0, 3.002, json!([{ "questID": 99196 }])), sample(3.0, 3.012, json!([{ "questID": 99197 }]))],
            }),
            vec![trace_event("GOSSIP_SHOW", 3.0, 3.001, &[]), trace_event("GOSSIP_SHOW", 3.0, 3.011, &[])],
        );
        assert_eq!(quests_and_values(&session), [(99196, Creature(276171)), (99197, Creature(276172))]);
    }
}
