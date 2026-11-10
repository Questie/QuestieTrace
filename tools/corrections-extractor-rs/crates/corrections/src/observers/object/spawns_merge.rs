//! The merges of object spawns and object zoneID. Port of
//! `extract/observers/object/spawnsMerge.ts`, which mirrors npc/spawnsMerge.ts and adds one
//! step: an object whose spawns in a zone all lie close together is one object seen from
//! different sides (a mailbox approached from all directions), so they collapse into one.
//!
//! Both receive, per object, its observed spawns followed by the quest turn-in POIs attached
//! to it (token `questpoi`).

use crate::js;
use crate::observation::Observation;
use crate::observers::npc::spawns_merge::{merge_spawns_with, Cluster, CLUSTER_EPSILON};
use crate::observers::spawn::SpawnObservation;

/// Identical to the npc merge in TS too.
pub use crate::observers::npc::spawns_merge::{merge_zone_id, Spawns};

/// Max distance between all spawns of a zone for them to be one object.
const OBJECT_COMPACT_EXTENT: f64 = 2.0;

/// The npc spawns merge, then per zone: when there are several clusters and all lie within
/// OBJECT_COMPACT_EXTENT of each other, one spawn at the mean of all their points.
pub fn merge_spawns(observations: &[Observation<SpawnObservation>], single_spawn_per_zone: bool) -> Spawns {
    merge_spawns_with(observations, single_spawn_per_zone, |clusters| {
        let centroids: Vec<[f64; 2]> = clusters.iter().map(|c| c.coord).collect();
        if clusters.len() <= 1 || !is_compact(&centroids) {
            return clusters;
        }
        let members: Vec<[f64; 2]> = clusters.into_iter().flat_map(|c| c.members).collect();
        let count = members.len() as f64;
        let sum_x = members.iter().fold(0.0, |sum, &[x, _]| sum + x);
        let sum_y = members.iter().fold(0.0, |sum, &[_, y]| sum + y);
        vec![Cluster { coord: [js::round((sum_x / count) * 100.0) / 100.0, js::round((sum_y / count) * 100.0) / 100.0], members }]
    })
}

fn is_compact(coords: &[[f64; 2]]) -> bool {
    coords.iter().enumerate().all(|(i, &[ax, ay])| coords[i + 1..].iter().all(|&[bx, by]| js_hypot(ax - bx, ay - by) <= OBJECT_COMPACT_EXTENT + CLUSTER_EPSILON))
}

/// JS `Math.hypot(a, b)` as V8 computes it (normalized Kahan sum); it differs from
/// `f64::hypot` in the last bit for about a third of inputs.
fn js_hypot(a: f64, b: f64) -> f64 {
    if a.is_infinite() || b.is_infinite() {
        return f64::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let (a, b) = (a.abs(), b.abs());
    let max = a.max(b);
    if max == 0.0 {
        return 0.0;
    }
    let (mut sum, mut compensation) = (0.0f64, 0.0f64);
    for value in [a, b] {
        let n = value / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::Confidence;
    use crate::observers::spawn::SpawnToken::{self, Npc, QuestNpc, QuestPoi, Target};

    fn obs(zone_id: f64, x: f64, y: f64, t: f64, token: SpawnToken) -> Observation<SpawnObservation> {
        Observation::new(2843, SpawnObservation { zone_id, x, y, token }, Confidence::High, t)
    }

    /// The zone's spawn points; empty when the zone is missing.
    fn zone(spawns: &Spawns, zone_id: f64) -> Vec<[f64; 2]> {
        spawns.iter().find(|(id, _)| *id == zone_id).map(|(_, points)| points.clone()).unwrap_or_default()
    }

    #[test]
    fn builds_the_zone_to_coordinates_table() {
        let observations = [obs(40.0, 30.01, 86.02, 10.0, Target), obs(40.0, 30.01, 86.02, 20.0, Target), obs(12.0, 47.46, 62.18, 30.0, Target)];
        assert_eq!(merge_spawns(&observations, false), [(12.0, vec![[47.46, 62.18]]), (40.0, vec![[30.01, 86.02]])]);
    }

    #[test]
    fn clusters_and_averages_nearby_coordinates() {
        // The duplicate (30.01, 86.02) is deduplicated first; (86.02 + 86.03) / 2 rounds to 86.03.
        let observations = [obs(40.0, 30.01, 86.02, 10.0, Target), obs(40.0, 30.01, 86.02, 20.0, Target), obs(40.0, 30.01, 86.03, 30.0, Target)];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[30.01, 86.03]])]);
    }

    #[test]
    fn spawns_of_no_observations_are_empty() {
        assert_eq!(merge_spawns(&[], false), []);
    }

    #[test]
    fn prefers_questnpc_over_target_for_the_same_coordinate() {
        let observations = [obs(40.0, 30.01, 86.02, 10.0, Target), obs(40.0, 30.01, 86.02, 20.0, QuestNpc)];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[30.01, 86.02]])]);
    }

    #[test]
    fn questnpc_overrides_all_other_coordinates() {
        let observations = [obs(40.0, 30.01, 86.02, 10.0, Target), obs(12.0, 47.46, 62.18, 20.0, Npc), obs(40.0, 50.0, 70.0, 30.0, QuestNpc)];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[50.0, 70.0]])]);
    }

    #[test]
    fn keeps_every_questnpc_coordinate() {
        let observations = [
            obs(40.0, 30.01, 86.02, 10.0, QuestNpc),
            obs(40.0, 35.0, 90.0, 20.0, QuestNpc),
            obs(12.0, 47.46, 62.18, 30.0, QuestNpc),
            obs(40.0, 30.01, 86.02, 40.0, Target),
        ];
        assert_eq!(merge_spawns(&observations, false), [(12.0, vec![[47.46, 62.18]]), (40.0, vec![[30.01, 86.02], [35.0, 90.0]])]);
    }

    #[test]
    fn clusters_a_tight_group_into_its_rounded_centroid() {
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
        let observations: Vec<_> = points.iter().enumerate().map(|(i, &(x, y))| obs(40.0, x, y, i as f64 + 1.0, QuestNpc)).collect();
        assert_eq!(zone(&merge_spawns(&observations, false), 40.0), [[21.85, 45.31]]);
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
        let observations = [obs(267.0, 62.52, 19.74, 1.0, Target), obs(267.0, 62.57, 19.9, 2.0, Target), obs(267.0, 62.59, 19.75, 3.0, Target)];
        assert_eq!(merge_spawns(&observations, false), [(267.0, vec![[62.56, 19.8]])]);
    }

    #[test]
    fn keeps_only_the_most_observed_cluster_per_zone_for_quest_entities() {
        let observations = [obs(40.0, 10.0, 50.0, 1.0, Target), obs(40.0, 30.0, 60.0, 2.0, Target), obs(40.0, 30.2, 60.0, 3.0, Target), obs(12.0, 47.46, 62.18, 4.0, Target)];
        assert_eq!(merge_spawns(&observations, true), [(12.0, vec![[47.46, 62.18]]), (40.0, vec![[30.1, 60.0]])]);
    }

    #[test]
    fn keeps_only_the_most_observed_questnpc_cluster_per_zone_for_quest_entities() {
        let observations = [obs(40.0, 10.0, 50.0, 1.0, QuestNpc), obs(40.0, 10.2, 50.0, 2.0, QuestNpc), obs(40.0, 30.0, 60.0, 3.0, QuestNpc)];
        assert_eq!(merge_spawns(&observations, true), [(40.0, vec![[10.1, 50.0]])]);
    }

    #[test]
    fn counts_repeated_observations_of_one_spot_when_picking_the_quest_entity_spawn() {
        let observations = [
            obs(40.0, 10.0, 50.0, 1.0, Target),
            obs(40.0, 10.0, 50.0, 2.0, Target),
            obs(40.0, 10.0, 50.0, 3.0, Target),
            obs(40.0, 30.0, 60.0, 4.0, Target),
            obs(40.0, 30.2, 60.0, 5.0, Target),
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
        let observations = [obs(40.0, 10.0, 50.0, 1.0, Target), obs(40.0, 10.3, 50.45, 2.0, Target), obs(40.0, 10.31, 50.0, 3.0, Target), obs(40.0, 10.32, 50.4, 4.0, Target)];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[10.23, 50.21]])]);
    }

    #[test]
    fn collapses_an_objects_spawns_into_one_when_they_all_lie_close_together() {
        let observations = [obs(1537.0, 50.0, 50.0, 1.0, Target), obs(1537.0, 51.0, 50.0, 2.0, Target), obs(1537.0, 50.5, 51.2, 3.0, Target)];
        assert_eq!(merge_spawns(&observations, false), [(1537.0, vec![[50.5, 50.4]])]);
    }

    #[test]
    fn keeps_an_objects_spawns_separate_when_they_are_spread_out() {
        let observations = [obs(40.0, 10.0, 10.0, 1.0, Target), obs(40.0, 10.0, 11.0, 2.0, Target), obs(40.0, 10.0, 12.1, 3.0, Target)];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[10.0, 10.0], [10.0, 11.0], [10.0, 12.1]])]);
    }

    #[test]
    fn uses_a_larger_grouping_radius_in_capital_cities() {
        let spread = |zone_id| [obs(zone_id, 50.0, 50.0, 1.0, Target), obs(zone_id, 50.8, 50.0, 2.0, Target), obs(zone_id, 52.6, 50.0, 3.0, Target), obs(zone_id, 53.4, 50.0, 4.0, Target)];
        assert_eq!(merge_spawns(&spread(1537.0), false), [(1537.0, vec![[50.4, 50.0], [53.0, 50.0]])]);
        assert_eq!(zone(&merge_spawns(&spread(40.0), false), 40.0).len(), 4);
    }

    #[test]
    fn does_not_chain_a_line_of_nearby_points_into_one_cluster() {
        let line = [10.0, 10.4, 10.8, 11.2, 11.6, 12.0, 12.4];
        let observations: Vec<_> = line.iter().enumerate().map(|(i, &x)| obs(40.0, x, 50.0, i as f64 + 1.0, Target)).collect();
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[10.2, 50.0], [11.0, 50.0], [11.8, 50.0], [12.4, 50.0]])]);
    }

    #[test]
    fn rounds_a_single_questnpc_coordinate_to_2_decimals() {
        assert_eq!(merge_spawns(&[obs(40.0, 30.123, 86.456, 1.0, QuestNpc)], false), [(40.0, vec![[30.12, 86.46]])]);
    }

    #[test]
    fn rounds_a_single_fallback_coordinate_to_2_decimals() {
        assert_eq!(merge_spawns(&[obs(40.0, 47.123, 62.456, 1.0, Target)], false), [(40.0, vec![[47.12, 62.46]])]);
    }

    #[test]
    fn handles_several_zones_with_several_coordinates_each() {
        let observations = [obs(40.0, 30.01, 86.02, 1.0, Target), obs(12.0, 47.46, 62.18, 2.0, Target), obs(40.0, 35.0, 90.0, 3.0, Target), obs(12.0, 52.0, 66.0, 4.0, Target)];
        assert_eq!(merge_spawns(&observations, false), [(12.0, vec![[47.46, 62.18], [52.0, 66.0]]), (40.0, vec![[30.01, 86.02], [35.0, 90.0]])]);
    }

    #[test]
    fn questpoi_overrides_questnpc_and_all_other_coordinates() {
        let observations = [obs(40.0, 30.01, 86.02, 10.0, Target), obs(12.0, 47.46, 62.18, 20.0, QuestNpc), obs(40.0, 50.0, 70.0, 30.0, QuestPoi)];
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[50.0, 70.0]])]);
    }

    #[test]
    fn zone_id_is_the_most_observed_zone() {
        let observations = [
            obs(40.0, 30.01, 86.02, 1.0, Target),
            obs(40.0, 31.0, 87.0, 2.0, Target),
            obs(12.0, 47.46, 62.18, 3.0, Target),
            obs(40.0, 32.0, 88.0, 4.0, Target),
            obs(12.0, 48.0, 63.0, 5.0, Target),
        ];
        assert_eq!(merge_zone_id(&observations), 40.0);
    }

    #[test]
    fn zone_id_of_no_observations_is_0() {
        assert_eq!(merge_zone_id(&[]), 0.0);
    }

    #[test]
    fn zone_id_ties_go_to_the_higher_token_priority() {
        assert_eq!(merge_zone_id(&[obs(12.0, 47.46, 62.18, 1.0, Target), obs(40.0, 30.01, 86.02, 2.0, Npc)]), 40.0);
    }

    #[test]
    fn zone_id_equal_weights_go_to_the_zone_seen_first() {
        assert_eq!(merge_zone_id(&[obs(12.0, 47.46, 62.18, 1.0, Target), obs(40.0, 30.01, 86.02, 2.0, Target)]), 12.0);
    }

    #[test]
    fn zone_id_weights_questnpc_above_two_targets() {
        let observations = [obs(12.0, 47.46, 62.18, 1.0, Target), obs(12.0, 48.0, 63.0, 2.0, Target), obs(40.0, 30.01, 86.02, 3.0, QuestNpc)];
        assert_eq!(merge_zone_id(&observations), 40.0);
    }

    #[test]
    fn zone_id_only_counts_questpoi_zones_when_questpoi_exists() {
        let observations = [obs(12.0, 1.0, 1.0, 10.0, QuestNpc), obs(12.0, 2.0, 2.0, 20.0, QuestNpc), obs(40.0, 50.0, 70.0, 30.0, QuestPoi)];
        assert_eq!(merge_zone_id(&observations), 40.0);
    }

    // Not in the TS tests: the JS behaviour the port has to reproduce.

    #[test]
    fn hypot_matches_v8_where_libm_differs() {
        // node: Math.hypot(-40.17, 61.260000000000005) === 73.25582911959975; f64::hypot gives ...77.
        assert_eq!(js_hypot(-40.17, 61.260000000000005), 73.25582911959975);
    }
}
