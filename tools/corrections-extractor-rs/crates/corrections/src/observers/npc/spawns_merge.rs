//! The merges of npc spawns and npc zoneID. Port of `extract/merge/npc/spawns.ts`.
//!
//! spawns: per zone, the observed coordinates clustered, each cluster placed at its median.
//! zoneID: the zone with the most observations, weighted by token.
//!
//! Both receive, per NPC, its observed spawns followed by the quest turn-in POIs attached to it
//! (token `questpoi`).

use crate::aggregate::Num;
use crate::observation::{Observation, SessionId};
use crate::observers::spawn::{
    best_coordinates_by_zone, cluster_coordinates, cluster_threshold, compare_coords, compare_numbers, coord_key, distinct_coordinates_by_zone,
    find_exclusive_token, in_js_property_order, round_2_decimals, CoordKey, SpawnObservation, SpawnToken,
};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};

/// Identical to the object merge in TS.
pub use crate::observers::spawn::{merge_zone_id, Spawns};

/// Tokens whose coordinates replace all other sources, in priority order.
const EXCLUSIVE_TOKENS: [SpawnToken; 2] = [SpawnToken::QuestPoi, SpawnToken::QuestNpc];

/// A cluster seen in a single session is only kept when it holds at least this share of the
/// sessions of the NPC's strongest cluster, so it only drops clusters once the strongest one has
/// more than 20 sessions. Drops stray points from targeting at range while NPCs seen only a few
/// times keep all their spawns.
const MIN_CLUSTER_SESSIONS: usize = 2;
const MIN_CLUSTER_SESSION_SHARE: f64 = 0.05;

/// Merges spawn observations into the Questie spawns table.
///
/// questpoi observations (quest turn-in POIs from C_QuestLog.GetQuestsOnMap) overrule all
/// others, then questnpc: when present, only that token's coordinates are kept. questnpc fires
/// during quest interactions, which need the player next to the NPC; target and npc can fire
/// from a distance. Without either, each coordinate keeps its highest-priority token. An NPC
/// with npc observations (an interaction window was open) then gets a single spawn per zone; in
/// zones where it was interacted with, only those positions are used.
///
/// Clusters seen in a single session are dropped when the NPC's strongest cluster was seen in
/// many (see MIN_CLUSTER_SESSION_SHARE). With `single_spawn_per_zone` (quest givers and enders,
/// which stand still) only the remaining cluster with the most observations is kept per zone.
pub fn merge_spawns(observations: &[Observation<SpawnObservation>], single_spawn_per_zone: bool) -> Spawns {
    let exclusive_token = find_exclusive_token(observations, &EXCLUSIVE_TOKENS);
    // An NPC the player opened an interaction window with (gossip, vendor, innkeeper, ...) has a
    // single spawn per zone: where it was interacted with, else its strongest cluster.
    let interaction_zones: HashSet<Num> = observations
        .iter()
        .filter(|obs| obs.value.token == SpawnToken::Npc)
        .map(|obs| Num(obs.value.zone_id))
        .collect();
    let is_used = |spawn: &SpawnObservation| match exclusive_token {
        Some(token) => spawn.token == token,
        None => spawn.token == SpawnToken::Npc || !interaction_zones.contains(&Num(spawn.zone_id)),
    };
    let used: Vec<&Observation<SpawnObservation>> = observations.iter().filter(|obs| is_used(&obs.value)).collect();

    let mut support: HashMap<CoordKey, CoordSupport> = HashMap::new();
    for obs in &used {
        let coord = support.entry(coord_key(obs.value.zone_id, obs.value.x, obs.value.y)).or_default();
        coord.observations += 1;
        coord.sessions.insert(obs.session);
    }

    let spawns = used.iter().map(|obs| &obs.value);
    match exclusive_token {
        Some(_) => cluster_by_zone(distinct_coordinates_by_zone(spawns), &support, single_spawn_per_zone),
        None => {
            let interactable = !interaction_zones.is_empty();
            cluster_by_zone(best_coordinates_by_zone(spawns), &support, single_spawn_per_zone || interactable)
        }
    }
}

/// What backs one coordinate key: its used observations and the sessions they came from.
#[derive(Default)]
struct CoordSupport {
    observations: usize,
    sessions: HashSet<SessionId>,
}

/// A cluster placed at its median, with the number of sessions that saw one of its points.
struct SupportedCluster {
    coord: [f64; 2],
    members: Vec<[f64; 2]>,
    sessions: usize,
}

/// Clusters each zone's distinct coordinates and drops weakly supported clusters. With
/// `single_spawn_per_zone`, only the remaining cluster with the most observations is kept per
/// zone; a zone without remaining clusters is left out.
fn cluster_by_zone(by_zone: IndexMap<Num, Vec<[f64; 2]>>, support: &HashMap<CoordKey, CoordSupport>, single_spawn_per_zone: bool) -> Spawns {
    // The strongest cluster of every zone, before any is dropped.
    let mut strongest = 0;
    let mut zone_clusters: Vec<(f64, Vec<SupportedCluster>)> = Vec::new();
    for (zone, coords) in by_zone {
        let zone_id = zone.0;
        let mut clusters = Vec::new();
        for cluster in cluster_coordinates(&coords, cluster_threshold(zone_id)) {
            let mut sessions = HashSet::new();
            for &[x, y] in &cluster.members {
                if let Some(coord) = support.get(&coord_key(zone_id, x, y)) {
                    sessions.extend(coord.sessions.iter().copied());
                }
            }
            strongest = strongest.max(sessions.len());
            // The median keeps a few far-off points from pulling the spawn away.
            let coord = [
                round_2_decimals(median(cluster.members.iter().map(|m| m[0]).collect())),
                round_2_decimals(median(cluster.members.iter().map(|m| m[1]).collect())),
            ];
            clusters.push(SupportedCluster {
                coord,
                members: cluster.members,
                sessions: sessions.len(),
            });
        }
        zone_clusters.push((zone_id, clusters));
    }

    let mut zones = Vec::new();
    for (zone_id, candidates) in zone_clusters {
        let mut clusters: Vec<SupportedCluster> = candidates
            .into_iter()
            .filter(|c| c.sessions >= MIN_CLUSTER_SESSIONS || c.sessions as f64 >= strongest as f64 * MIN_CLUSTER_SESSION_SHARE)
            .collect();
        if clusters.is_empty() {
            continue;
        }
        if single_spawn_per_zone {
            let weight = |members: &[[f64; 2]]| -> usize {
                members
                    .iter()
                    .map(|&[x, y]| support.get(&coord_key(zone_id, x, y)).map_or(1, |coord| coord.observations))
                    .sum()
            };
            // `reduce`: the first cluster of the highest weight wins.
            let mut best = 0;
            for index in 1..clusters.len() {
                if weight(&clusters[index].members) > weight(&clusters[best].members) {
                    best = index;
                }
            }
            clusters = vec![clusters.swap_remove(best)];
        }
        let mut spawns: Vec<[f64; 2]> = clusters.into_iter().map(|c| c.coord).collect();
        spawns.sort_by(compare_coords);
        zones.push((zone_id, spawns));
    }
    in_js_property_order(zones)
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| compare_numbers(*a, *b));
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::Confidence;
    use SpawnToken::{Npc, QuestNpc, QuestPoi, Target};

    /// An observation of npc 823. Observations left without a session all count as one.
    fn obs(zone_id: f64, x: f64, y: f64, t: f64, token: SpawnToken) -> Observation<SpawnObservation> {
        Observation::new(823, SpawnObservation { zone_id, x, y, token }, Confidence::High, t)
    }

    /// `obs` seen in its own session (TS sets `provenance.sessionKey`).
    fn obs_in(session: u32, zone_id: f64, x: f64, y: f64, t: f64, token: SpawnToken) -> Observation<SpawnObservation> {
        Observation {
            session: SessionId(session),
            ..obs(zone_id, x, y, t, token)
        }
    }

    /// The zone's spawn points; empty when the zone is missing.
    fn zone(spawns: &Spawns, zone_id: f64) -> Vec<[f64; 2]> {
        spawns
            .iter()
            .find(|(id, _)| *id == zone_id)
            .map(|(_, points)| points.clone())
            .unwrap_or_default()
    }

    #[test]
    fn builds_the_zone_to_coordinates_table() {
        let observations = [
            obs(40.0, 30.01, 86.02, 10.0, Npc),
            obs(40.0, 30.01, 86.02, 20.0, Npc),
            obs(12.0, 47.46, 62.18, 30.0, Npc),
        ];
        assert_eq!(merge_spawns(&observations, false), [(12.0, vec![[47.46, 62.18]]), (40.0, vec![[30.01, 86.02]])]);
    }

    #[test]
    fn clusters_and_averages_nearby_coordinates() {
        // The duplicate (30.01, 86.02) is deduplicated first; (86.02 + 86.03) / 2 rounds to 86.03.
        let observations = [
            obs(40.0, 30.01, 86.02, 10.0, Npc),
            obs(40.0, 30.01, 86.02, 20.0, Npc),
            obs(40.0, 30.01, 86.03, 30.0, Npc),
        ];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[30.01, 86.03]])]);
    }

    #[test]
    fn prefers_questnpc_over_target_for_the_same_coordinate() {
        let observations = [obs(40.0, 30.01, 86.02, 10.0, Target), obs(40.0, 30.01, 86.02, 20.0, QuestNpc)];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[30.01, 86.02]])]);
    }

    #[test]
    fn questnpc_overrides_all_other_coordinates() {
        let observations = [
            obs(40.0, 30.01, 86.02, 10.0, Target),
            obs(12.0, 47.46, 62.18, 20.0, Npc),
            obs(40.0, 50.0, 70.0, 30.0, QuestNpc),
        ];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[50.0, 70.0]])]);
    }

    #[test]
    fn keeps_every_questnpc_coordinate() {
        let observations = [
            obs(40.0, 30.01, 86.02, 10.0, QuestNpc),
            obs(40.0, 31.0, 87.0, 20.0, QuestNpc),
            obs(12.0, 47.46, 62.18, 30.0, QuestNpc),
            obs(40.0, 30.01, 86.02, 40.0, Target),
        ];
        assert_eq!(
            merge_spawns(&observations, false),
            [(12.0, vec![[47.46, 62.18]]), (40.0, vec![[30.01, 86.02], [31.0, 87.0]])]
        );
    }

    #[test]
    fn clusters_nearby_coordinates_into_their_median() {
        let points = [
            (21.83, 45.3),
            (21.83, 45.32),
            (21.83, 45.35),
            (21.84, 45.26),
            (21.84, 45.29),
            (21.84, 45.3),
            (21.84, 45.34),
            (21.84, 45.35),
            (21.86, 45.28),
            (21.86, 45.3),
            (21.87, 45.29),
            (21.87, 45.3),
            (21.88, 45.31),
        ];
        let observations: Vec<_> = points
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| obs(40.0, x, y, i as f64 + 1.0, QuestNpc))
            .collect();
        assert_eq!(zone(&merge_spawns(&observations, false), 40.0), [[21.84, 45.3]]);
    }

    #[test]
    fn keeps_separate_clusters_for_points_far_apart() {
        let observations = [
            obs(40.0, 30.0, 86.0, 1.0, QuestNpc),
            obs(40.0, 30.02, 86.01, 2.0, QuestNpc),
            obs(40.0, 30.04, 86.0, 3.0, QuestNpc),
            obs(40.0, 47.46, 62.18, 4.0, QuestNpc),
            obs(40.0, 47.48, 62.2, 5.0, QuestNpc),
        ];
        assert_eq!(zone(&merge_spawns(&observations, false), 40.0).len(), 2);
    }

    #[test]
    fn collapses_observations_around_a_static_entity_into_one_spawn() {
        let observations = [
            obs(267.0, 62.52, 19.74, 1.0, Target),
            obs(267.0, 62.57, 19.9, 2.0, Target),
            obs(267.0, 62.59, 19.75, 3.0, Target),
        ];
        assert_eq!(merge_spawns(&observations, false), [(267.0, vec![[62.57, 19.75]])]);
    }

    #[test]
    fn keeps_only_the_most_observed_cluster_per_zone_for_quest_entities() {
        let observations = [
            obs(40.0, 10.0, 50.0, 1.0, Npc),
            obs(40.0, 30.0, 60.0, 2.0, Npc),
            obs(40.0, 30.2, 60.0, 3.0, Npc),
            obs(12.0, 47.46, 62.18, 4.0, Npc),
        ];
        assert_eq!(merge_spawns(&observations, true), [(12.0, vec![[47.46, 62.18]]), (40.0, vec![[30.1, 60.0]])]);
    }

    #[test]
    fn keeps_only_the_most_observed_questnpc_cluster_per_zone_for_quest_entities() {
        let observations = [
            obs(40.0, 10.0, 50.0, 1.0, QuestNpc),
            obs(40.0, 10.2, 50.0, 2.0, QuestNpc),
            obs(40.0, 30.0, 60.0, 3.0, QuestNpc),
        ];
        assert_eq!(merge_spawns(&observations, true), [(40.0, vec![[10.1, 50.0]])]);
    }

    #[test]
    fn counts_repeated_observations_of_one_spot_when_picking_the_quest_entity_spawn() {
        let observations = [
            obs(40.0, 10.0, 50.0, 1.0, Npc),
            obs(40.0, 10.0, 50.0, 2.0, Npc),
            obs(40.0, 10.0, 50.0, 3.0, Npc),
            obs(40.0, 30.0, 60.0, 4.0, Npc),
            obs(40.0, 30.2, 60.0, 5.0, Npc),
        ];
        assert_eq!(merge_spawns(&observations, true), [(40.0, vec![[10.0, 50.0]])]);
    }

    #[test]
    fn keeps_every_cluster_for_entities_that_are_not_quest_entities() {
        let observations = [obs(40.0, 10.0, 50.0, 1.0, Target), obs(40.0, 30.0, 60.0, 2.0, Target)];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[10.0, 50.0], [30.0, 60.0]])]);
    }

    #[test]
    fn merges_clusters_whose_centroids_end_up_within_the_threshold() {
        let observations = [
            obs(40.0, 10.0, 50.0, 1.0, Npc),
            obs(40.0, 10.3, 50.45, 2.0, Npc),
            obs(40.0, 10.31, 50.0, 3.0, Npc),
            obs(40.0, 10.32, 50.4, 4.0, Npc),
        ];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[10.31, 50.2]])]);
    }

    #[test]
    fn uses_a_larger_grouping_radius_in_capital_cities() {
        let spread = |zone_id| [obs(zone_id, 50.0, 50.0, 1.0, Target), obs(zone_id, 50.8, 50.0, 2.0, Target)];
        assert_eq!(merge_spawns(&spread(1537.0), false), [(1537.0, vec![[50.4, 50.0]])]);
        assert_eq!(zone(&merge_spawns(&spread(40.0), false), 40.0).len(), 2);
    }

    #[test]
    fn does_not_chain_a_line_of_nearby_points_into_one_cluster() {
        let observations: Vec<_> = [10.0, 10.4, 10.8, 11.2, 11.6]
            .iter()
            .enumerate()
            .map(|(i, &x)| obs(40.0, x, 50.0, i as f64 + 1.0, Target))
            .collect();
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[10.2, 50.0], [11.0, 50.0], [11.6, 50.0]])]);
    }

    #[test]
    fn clusters_points_exactly_at_the_threshold_distance() {
        let observations = [obs(40.0, 30.0, 86.0, 1.0, QuestNpc), obs(40.0, 30.5, 86.0, 2.0, QuestNpc)];
        assert_eq!(zone(&merge_spawns(&observations, false), 40.0).len(), 1);
    }

    #[test]
    fn does_not_cluster_points_just_beyond_the_threshold() {
        let observations = [obs(40.0, 30.0, 86.0, 1.0, QuestNpc), obs(40.0, 30.51, 86.0, 2.0, QuestNpc)];
        assert_eq!(zone(&merge_spawns(&observations, false), 40.0).len(), 2);
    }

    #[test]
    fn rounds_a_single_questnpc_coordinate_to_2_decimals() {
        assert_eq!(merge_spawns(&[obs(40.0, 30.123, 86.456, 1.0, QuestNpc)], false), [(40.0, vec![[30.12, 86.46]])]);
    }

    #[test]
    fn rounds_a_single_fallback_coordinate_to_2_decimals() {
        assert_eq!(merge_spawns(&[obs(40.0, 47.123, 62.456, 1.0, Npc)], false), [(40.0, vec![[47.12, 62.46]])]);
    }

    #[test]
    fn zone_id_weights_questnpc_above_two_targets() {
        // Zone 40 weighs 3 (questnpc), zone 12 weighs 2 (two targets).
        let observations = [
            obs(12.0, 47.46, 62.18, 1.0, Target),
            obs(12.0, 48.0, 63.0, 2.0, Target),
            obs(40.0, 30.01, 86.02, 3.0, QuestNpc),
        ];
        assert_eq!(merge_zone_id(&observations), 40.0);
    }

    #[test]
    fn drops_a_single_session_cluster_when_the_strongest_cluster_has_many_sessions() {
        let mut observations: Vec<_> = (0..25).map(|i| obs_in(i, 40.0, 30.0, 30.0, i as f64, Target)).collect();
        observations.push(obs_in(99, 40.0, 60.0, 60.0, 99.0, Target));
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[30.0, 30.0]])]);
    }

    #[test]
    fn keeps_a_cluster_seen_in_two_sessions() {
        let mut observations: Vec<_> = (0..25).map(|i| obs_in(i, 40.0, 30.0, 30.0, i as f64, Target)).collect();
        observations.push(obs_in(98, 12.0, 60.0, 60.0, 98.0, Target));
        observations.push(obs_in(99, 12.0, 60.0, 60.0, 99.0, Target));
        assert_eq!(merge_spawns(&observations, false), [(12.0, vec![[60.0, 60.0]]), (40.0, vec![[30.0, 30.0]])]);
    }

    #[test]
    fn picks_the_single_quest_entity_spawn_among_well_supported_clusters() {
        // The cluster at (60, 60) has more observations, but all from one session.
        let mut observations: Vec<_> = (0..25).map(|i| obs_in(i, 40.0, 30.0, 30.0, i as f64, Npc)).collect();
        observations.extend((0..50).map(|i| obs_in(100, 40.0, 60.0, 60.0 + i as f64 * 0.001, 100.0 + i as f64, Npc)));
        assert_eq!(merge_spawns(&observations, true), [(40.0, vec![[30.0, 30.0]])]);
    }

    #[test]
    fn keeps_single_session_clusters_of_rarely_seen_npcs() {
        let observations = [obs_in(1, 40.0, 30.0, 30.0, 1.0, Target), obs_in(2, 40.0, 60.0, 60.0, 2.0, Target)];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[30.0, 30.0], [60.0, 60.0]])]);
    }

    #[test]
    fn places_a_cluster_at_the_median_so_an_outlier_does_not_pull_it_away() {
        let observations = [
            obs(40.0, 30.0, 30.0, 1.0, Target),
            obs(40.0, 30.1, 30.1, 2.0, Target),
            obs(40.0, 30.2, 30.2, 3.0, Target),
            obs(40.0, 30.4, 30.4, 4.0, Target),
        ];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[30.15, 30.15]])]);
    }

    #[test]
    fn spawns_of_no_observations_are_empty() {
        assert_eq!(merge_spawns(&[], false), []);
    }

    #[test]
    fn handles_several_zones_with_several_coordinates_each() {
        let observations = [
            obs(40.0, 30.01, 86.02, 1.0, Target),
            obs(12.0, 47.46, 62.18, 2.0, Target),
            obs(40.0, 31.0, 87.0, 3.0, Target),
            obs(12.0, 48.0, 63.0, 4.0, Target),
        ];
        assert_eq!(
            merge_spawns(&observations, false),
            [(12.0, vec![[47.46, 62.18], [48.0, 63.0]]), (40.0, vec![[30.01, 86.02], [31.0, 87.0]])]
        );
    }

    #[test]
    fn reduces_an_interacted_npc_to_a_single_spawn_per_zone_at_its_interaction_positions() {
        let observations = [
            obs(40.0, 50.0, 50.0, 1.0, Npc),
            obs(40.0, 50.2, 50.0, 2.0, Npc),
            obs(40.0, 52.0, 50.0, 3.0, Npc),
            obs(40.0, 58.0, 45.0, 4.0, Target),
            obs(40.0, 58.0, 45.0, 5.0, Target),
            obs(40.0, 58.0, 45.0, 6.0, Target),
            obs(12.0, 20.0, 20.0, 7.0, Npc),
        ];
        assert_eq!(merge_spawns(&observations, false), [(12.0, vec![[20.0, 20.0]]), (40.0, vec![[50.1, 50.0]])]);
    }

    #[test]
    fn does_not_let_ignored_targets_add_support_to_an_interaction_cluster() {
        let observations = [
            obs(40.0, 50.0, 50.0, 1.0, Npc),
            obs(40.0, 50.0, 50.0, 2.0, Npc),
            obs(40.0, 60.0, 60.0, 3.0, Npc),
            obs(40.0, 60.0, 60.0, 4.0, Target),
            obs(40.0, 60.0, 60.0, 5.0, Target),
            obs(40.0, 60.0, 60.0, 6.0, Target),
        ];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[50.0, 50.0]])]);
    }

    #[test]
    fn reduces_an_interacted_npcs_targets_in_a_zone_without_interactions_to_a_single_spawn() {
        let observations = [
            obs(40.0, 50.0, 50.0, 1.0, Npc),
            obs(12.0, 20.0, 20.0, 2.0, Target),
            obs(12.0, 20.1, 20.0, 3.0, Target),
            obs(12.0, 30.0, 30.0, 4.0, Target),
        ];
        assert_eq!(merge_spawns(&observations, false), [(12.0, vec![[20.05, 20.0]]), (40.0, vec![[50.0, 50.0]])]);
    }

    #[test]
    fn questpoi_overrides_questnpc_and_all_other_coordinates() {
        let observations = [
            obs(40.0, 30.01, 86.02, 10.0, Target),
            obs(12.0, 47.46, 62.18, 20.0, QuestNpc),
            obs(40.0, 50.0, 70.0, 30.0, QuestPoi),
        ];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[50.0, 70.0]])]);
    }

    // describe("mergeZoneID")

    #[test]
    fn zone_id_is_the_most_observed_zone() {
        let observations = [
            obs(40.0, 30.01, 86.02, 1.0, Npc),
            obs(40.0, 31.0, 87.0, 2.0, Npc),
            obs(12.0, 47.46, 62.18, 3.0, Npc),
            obs(40.0, 32.0, 88.0, 4.0, Npc),
            obs(12.0, 48.0, 63.0, 5.0, Npc),
        ];
        assert_eq!(merge_zone_id(&observations), 40.0);
    }

    #[test]
    fn zone_id_ties_go_to_the_higher_token_priority() {
        // Zone 40 (npc, 2) beats zone 12 (target, 1).
        assert_eq!(merge_zone_id(&[obs(12.0, 47.46, 62.18, 1.0, Target), obs(40.0, 30.01, 86.02, 2.0, Npc)]), 40.0);
    }

    #[test]
    fn zone_id_equal_weights_go_to_the_zone_seen_first() {
        assert_eq!(merge_zone_id(&[obs(12.0, 47.46, 62.18, 1.0, Npc), obs(40.0, 30.01, 86.02, 2.0, Npc)]), 12.0);
    }

    #[test]
    fn zone_id_only_counts_questpoi_zones_when_questpoi_exists() {
        let observations = [
            obs(12.0, 1.0, 1.0, 10.0, QuestNpc),
            obs(12.0, 2.0, 2.0, 20.0, QuestNpc),
            obs(40.0, 50.0, 70.0, 30.0, QuestPoi),
        ];
        assert_eq!(merge_zone_id(&observations), 40.0);
    }

    // Not in the TS tests: the JS behaviour the port has to reproduce.

    #[test]
    fn zones_come_out_in_js_property_order() {
        let observations = [
            obs(1519.0, 1.0, 1.0, 1.0, Target),
            obs(-1.0, 2.0, 2.0, 2.0, Target),
            obs(12.0, 3.0, 3.0, 3.0, Target),
            obs(0.5, 4.0, 4.0, 4.0, Target),
        ];
        let zones: Vec<f64> = merge_spawns(&observations, false).into_iter().map(|(zone, _)| zone).collect();
        assert_eq!(zones, [12.0, 1519.0, -1.0, 0.5]);
    }
}
