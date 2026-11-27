//! npcKeys.rank - Questie field 6 (int, default 0; see
//! https://github.com/cmangos/issues/wiki/creature_template#rank). Port of
//! `extract/observers/npc/rank.ts`.
//!
//! UnitClassification[token] from Modules/Trackers/UnitState.lua, paired with the NPC's GUID
//! (see unit_state.rs).
//!
//! The classification is static, so sessions can only disagree through a wrong pairing. The
//! rank seen in the most sessions wins; a tie or a rank seen in only one session gives no
//! value. Rank 0 is not exported, since it adds nothing over nil.

use crate::observation::{Confidence, Observation, SessionId};
use crate::observers::npc::unit_state::unit_state_encounters;
use indexmap::{IndexMap, IndexSet};
use rustc_hash::FxHashSet;
use std::cmp::Reverse;
use trace_data::Session;

fn rank_by_classification(classification: &str) -> Option<i64> {
    match classification {
        "normal" | "trivial" | "minus" => Some(0),
        "elite" => Some(1),
        "rareelite" => Some(2),
        "worldboss" => Some(3),
        "rare" => Some(4),
        _ => None,
    }
}

const MIN_SESSIONS: usize = 2;

/// Each rank of each NPC once per session.
pub fn observe(session: &Session) -> Vec<Observation<i64>> {
    let mut observations = Vec::new();
    let mut seen: FxHashSet<(i64, i64)> = FxHashSet::default();
    for encounter in unit_state_encounters(session, "UnitClassification", &[]) {
        let Some(rank) = encounter.value.as_str().and_then(rank_by_classification) else {
            continue;
        };
        if !seen.insert((encounter.npc_id, rank)) {
            continue;
        }
        observations.push(Observation::new(encounter.npc_id, rank, Confidence::High, encounter.t));
    }
    observations
}

/// None when no rank is reliable enough to export, or the rank is 0.
pub fn merge(observations: &[Observation<i64>]) -> Option<i64> {
    let mut sessions_by_rank: IndexMap<i64, IndexSet<SessionId>> = IndexMap::new();
    for obs in observations {
        sessions_by_rank.entry(obs.value).or_default().insert(obs.session);
    }
    let mut counts: Vec<(i64, usize)> = sessions_by_rank.iter().map(|(&rank, sessions)| (rank, sessions.len())).collect();
    // Stable, like JS sort: equal counts keep their first-seen order.
    counts.sort_by_key(|&(_, count)| Reverse(count));
    let (best_rank, best_count) = *counts.first()?;
    let second_count = counts.get(1).map(|&(_, count)| count);
    if best_count < MIN_SESSIONS || Some(best_count) == second_count {
        return None;
    }
    (best_rank != 0).then_some(best_rank)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{event, packed, session_json, with_session};
    use serde_json::{json, Value as Json};

    fn creature_guid(id: i64) -> String {
        format!("Creature-0-5208-0-7-{id}-000032")
    }

    /// TS `apiSample`: a stream entry; a null value is recorded as nil.
    fn sample(t: f64, v: Json) -> Json {
        if v.is_null() {
            json!({ "t": t, "tp": t })
        } else {
            json!({ "t": t, "tp": t, "v": v })
        }
    }

    fn observed(functions: Json, events: Vec<Json>) -> Vec<(i64, i64, Confidence, f64)> {
        with_session(&session_json(functions, events), |s| {
            observe(s).into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect()
        })
    }

    fn vote(session: u32, rank: i64) -> Observation<i64> {
        Observation {
            session: SessionId(session),
            ..Observation::new(1, rank, Confidence::High, 0.0)
        }
    }

    #[test]
    fn maps_each_classification_to_its_rank() {
        let cases = [
            ("normal", 0),
            ("trivial", 0),
            ("minus", 0),
            ("elite", 1),
            ("rareelite", 2),
            ("worldboss", 3),
            ("rare", 4),
        ];
        for (classification, rank) in cases {
            let functions = json!({
                "UnitGUID": { "target": [sample(5.0, creature_guid(200001).into())] },
                "UnitClassification": { "target": [sample(5.0, classification.into())] },
            });
            assert_eq!(observed(functions, vec![]), [(200001, rank, Confidence::High, 5.0)], "{classification}");
        }
    }

    #[test]
    fn skips_unknown_classifications() {
        let functions = json!({
            "UnitGUID": { "target": [sample(5.0, creature_guid(200001).into())] },
            "UnitClassification": { "target": [sample(5.0, "unknown".into())] },
        });
        assert_eq!(observed(functions, vec![]), []);
    }

    #[test]
    fn does_not_pair_a_stale_classification_with_a_guid_sampled_at_another_event() {
        let functions = json!({
            "UnitGUID": { "target": [sample(3.0, creature_guid(200001).into()), sample(8.0, creature_guid(200002).into())] },
            "UnitClassification": { "target": [sample(3.0, "elite".into())] },
        });
        let events = vec![event(3.0, "PLAYER_TARGET_CHANGED", packed(&[])), event(8.0, "QUEST_DETAIL", packed(&[]))];
        let ids: Vec<i64> = observed(functions, events).into_iter().map(|(id, ..)| id).collect();
        assert_eq!(ids, [200001]);
    }

    #[test]
    fn records_one_observation_per_npc_and_rank_in_a_session() {
        let functions = json!({
            "UnitGUID": { "target": [sample(3.0, creature_guid(200001).into()), sample(4.0, Json::Null), sample(5.0, creature_guid(200001).into())] },
            "UnitClassification": { "target": [sample(3.0, "elite".into()), sample(4.0, "normal".into()), sample(5.0, "elite".into())] },
        });
        let ranks: Vec<(i64, i64)> = observed(functions, vec![]).into_iter().map(|(id, rank, ..)| (id, rank)).collect();
        assert_eq!(ranks, [(200001, 1)]);
    }

    #[test]
    fn merge_returns_the_rank_seen_in_the_most_sessions() {
        assert_eq!(merge(&[vote(1, 1), vote(2, 1), vote(3, 4)]), Some(1));
    }

    #[test]
    fn merge_counts_several_observations_in_one_session_once() {
        assert_eq!(merge(&[vote(1, 4), vote(1, 4), vote(1, 4), vote(2, 1), vote(3, 1)]), Some(1));
    }

    #[test]
    fn merge_gives_nothing_when_the_most_common_ranks_tie() {
        assert_eq!(merge(&[vote(1, 1), vote(2, 1), vote(3, 4), vote(4, 4)]), None);
    }

    #[test]
    fn merge_gives_nothing_for_a_rank_seen_in_only_one_session() {
        assert_eq!(merge(&[vote(1, 1), vote(1, 1)]), None);
    }

    #[test]
    fn merge_gives_nothing_for_rank_0() {
        assert_eq!(merge(&[vote(1, 0), vote(2, 0)]), None);
    }
}
