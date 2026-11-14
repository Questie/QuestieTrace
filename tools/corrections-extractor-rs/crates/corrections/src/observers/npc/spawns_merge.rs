//! The merges of npc spawns and npc zoneID. Port of `extract/observers/npc/spawnsMerge.ts`.
//!
//! spawns: per zone, the observed coordinates clustered and averaged.
//! zoneID: the zone with the most observations, weighted by token.
//!
//! Both receive, per NPC, its observed spawns followed by the quest turn-in POIs attached to
//! it (token `questpoi`). The object merges (object/spawns_merge.rs) reuse this module: their
//! TS twin mirrors this one, except that it collapses compact clusters.

use crate::aggregate::Num;
use crate::js;
use crate::observation::Observation;
use crate::observers::spawn::{SpawnObservation, SpawnToken};
use indexmap::map::Entry;
use indexmap::IndexMap;
use std::cmp::Ordering;
use std::collections::HashMap;

/// A spawns table `{[zoneID]: [[x, y], ...]}`: per zone (AreaTable areaID) its spawn points in
/// Questie 0-100 map coordinates. Zones are in JS property order (integer ids ascending), the
/// order `Object.entries` gives the Lua writer.
pub type Spawns = Vec<(f64, Vec<[f64; 2]>)>;

fn token_priority(token: SpawnToken) -> u32 {
    match token {
        SpawnToken::QuestPoi => 4,
        SpawnToken::QuestNpc => 3,
        SpawnToken::Npc => 2,
        SpawnToken::Target => 1,
    }
}

/// Max distance (Questie 0-100 map units) from a cluster's centroid. Covers the spread of
/// recording the player's position within interaction range.
const SPAWN_CLUSTER_THRESHOLD: f64 = 0.5;
/// Capital city maps are small, so the same interaction range covers more map units.
const CAPITAL_CITY_CLUSTER_THRESHOLD: f64 = 1.0;
/// Stormwind, Ironforge, Darnassus, Orgrimmar, Thunder Bluff, Undercity.
const CAPITAL_CITY_ZONE_IDS: [f64; 6] = [1519.0, 1537.0, 1657.0, 1637.0, 1638.0, 1497.0];
/// Tolerance for floating point comparisons.
pub(crate) const CLUSTER_EPSILON: f64 = 1e-9;

/// Tokens whose coordinates replace all other sources, in priority order.
const EXCLUSIVE_TOKENS: [SpawnToken; 2] = [SpawnToken::QuestPoi, SpawnToken::QuestNpc];

fn find_exclusive_token(observations: &[Observation<SpawnObservation>]) -> Option<SpawnToken> {
    EXCLUSIVE_TOKENS
        .into_iter()
        .find(|&token| observations.iter().any(|obs| obs.value.token == token))
}

/// Merges spawn observations into the Questie spawns table.
///
/// questpoi observations (quest turn-in POIs from C_QuestLog.GetQuestsOnMap) overrule all
/// others, then questnpc: when present, only that token's coordinates are kept. questnpc fires
/// during quest interactions, which need the player next to the NPC; target and npc can fire
/// from a distance. Without either, each coordinate keeps its highest-priority token.
///
/// With `single_spawn_per_zone` (quest givers and enders, which stand still) only the cluster
/// with the most observations is kept per zone.
pub fn merge_spawns(observations: &[Observation<SpawnObservation>], single_spawn_per_zone: bool) -> Spawns {
    merge_spawns_with(observations, single_spawn_per_zone, |clusters| clusters)
}

/// The zone the most observations were made in: questnpc counts 3, npc 2 and target 1. When
/// questpoi observations exist only they count. 0 (the schema default) without observations.
pub fn merge_zone_id(observations: &[Observation<SpawnObservation>]) -> f64 {
    if observations.is_empty() {
        return 0.0;
    }
    let has_questpoi = observations.iter().any(|obs| obs.value.token == SpawnToken::QuestPoi);
    let mut zone_weights: IndexMap<Num, u32> = IndexMap::new();
    for obs in observations {
        if has_questpoi && obs.value.token != SpawnToken::QuestPoi {
            continue;
        }
        // A JS Map stores a -0 key as +0.
        *zone_weights.entry(Num(obs.value.zone_id + 0.0)).or_insert(0) += token_priority(obs.value.token);
    }
    let (mut best_zone, mut best_weight) = (0.0, 0);
    for (zone, weight) in zone_weights {
        if weight > best_weight {
            best_weight = weight;
            best_zone = zone.0;
        }
    }
    best_zone
}

/// A cluster of nearby coordinates: its centroid rounded to 2 decimals, and its points.
pub(crate) struct Cluster {
    pub coord: [f64; 2],
    pub members: Vec<[f64; 2]>,
}

/// mergeSpawns, with `refine` applied to each zone's clusters before they are sorted into
/// spawn points. The object merge collapses compact clusters there.
pub(crate) fn merge_spawns_with(
    observations: &[Observation<SpawnObservation>],
    single_spawn_per_zone: bool,
    refine: impl Fn(Vec<Cluster>) -> Vec<Cluster>,
) -> Spawns {
    let exclusive_token = find_exclusive_token(observations);
    let mut observation_counts: HashMap<CoordKey, usize> = HashMap::new();
    for obs in observations {
        if exclusive_token.is_some_and(|token| obs.value.token != token) {
            continue;
        }
        *observation_counts.entry(coord_key(obs.value.zone_id, obs.value.x, obs.value.y)).or_insert(0) += 1;
    }

    // Zones in the order first seen; a JS Map stores a -0 key as +0.
    let mut by_zone: IndexMap<Num, Vec<[f64; 2]>> = IndexMap::new();
    if let Some(exclusive_token) = exclusive_token {
        for obs in observations.iter().filter(|obs| obs.value.token == exclusive_token) {
            let SpawnObservation { zone_id, x, y, .. } = obs.value;
            let coords = by_zone.entry(Num(zone_id + 0.0)).or_default();
            if !coords.iter().any(|c| c[0] == x && c[1] == y) {
                coords.push([x, y]);
            }
        }
    } else {
        // Per zone and coordinate, the observation with the highest token priority. A replaced
        // observation keeps the position of the first one, as Map.set does.
        let mut best_per_coord: IndexMap<CoordKey, &SpawnObservation> = IndexMap::new();
        for obs in observations {
            let SpawnObservation { zone_id, x, y, token } = obs.value;
            match best_per_coord.entry(coord_key(zone_id, x, y)) {
                Entry::Occupied(mut best) => {
                    if token_priority(token) > token_priority(best.get().token) {
                        best.insert(&obs.value);
                    }
                }
                Entry::Vacant(slot) => {
                    slot.insert(&obs.value);
                }
            }
        }
        for spawn in best_per_coord.values() {
            by_zone.entry(Num(spawn.zone_id + 0.0)).or_default().push([spawn.x, spawn.y]);
        }
    }

    let zones = by_zone
        .into_iter()
        .map(|(zone, coords)| {
            let zone_id = zone.0;
            let threshold = if CAPITAL_CITY_ZONE_IDS.contains(&zone_id) {
                CAPITAL_CITY_CLUSTER_THRESHOLD
            } else {
                SPAWN_CLUSTER_THRESHOLD
            };
            let mut clusters = cluster_and_average_coordinates(&coords, threshold);
            if single_spawn_per_zone {
                let weight = |members: &[[f64; 2]]| -> usize {
                    members
                        .iter()
                        .map(|&[x, y]| observation_counts.get(&coord_key(zone_id, x, y)).copied().unwrap_or(1))
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
            let mut spawns: Vec<[f64; 2]> = refine(clusters).into_iter().map(|c| c.coord).collect();
            spawns.sort_by(compare_coords);
            (zone_id, spawns)
        })
        .collect();
    in_js_property_order(zones)
}

/// `(a, b) => a[0] - b[0] || a[1] - b[1]`, read the way JS sort reads it (NaN is equal).
fn compare_coords(a: &[f64; 2], b: &[f64; 2]) -> Ordering {
    let dx = a[0] - b[0];
    let difference = if dx == 0.0 || dx.is_nan() { a[1] - b[1] } else { dx };
    if difference < 0.0 {
        Ordering::Less
    } else if difference > 0.0 {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// A cluster while points join it.
struct GrowingCluster {
    members: Vec<[f64; 2]>,
    count: usize,
    sum_x: f64,
    sum_y: f64,
}

impl GrowingCluster {
    fn centroid(&self) -> [f64; 2] {
        [self.sum_x / self.count as f64, self.sum_y / self.count as f64]
    }
}

/// Clusters nearby coordinates and averages each cluster. A point joins the nearest cluster
/// whose centroid is within `threshold`, which avoids chaining a line of points (a patrol)
/// into one cluster.
fn cluster_and_average_coordinates(coords: &[[f64; 2]], threshold: f64) -> Vec<Cluster> {
    // Sorted first for a consistent clustering order.
    let mut coords = coords.to_vec();
    coords.sort_by(compare_coords);

    let mut clusters: Vec<GrowingCluster> = Vec::new();
    for [x, y] in coords {
        let mut nearest = None;
        let mut nearest_dist = f64::INFINITY;
        for (index, cluster) in clusters.iter().enumerate() {
            let [cx, cy] = cluster.centroid();
            let (dx, dy) = (x - cx, y - cy);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < nearest_dist {
                nearest_dist = dist;
                nearest = Some(index);
            }
        }
        match nearest {
            Some(index) if nearest_dist <= threshold + CLUSTER_EPSILON => {
                let cluster = &mut clusters[index];
                cluster.members.push([x, y]);
                cluster.count += 1;
                cluster.sum_x += x;
                cluster.sum_y += y;
            }
            _ => clusters.push(GrowingCluster {
                members: vec![[x, y]],
                count: 1,
                sum_x: x,
                sum_y: y,
            }),
        }
    }

    merge_close_clusters(&mut clusters, threshold);

    // Each centroid rounded to 2 decimals.
    clusters
        .into_iter()
        .map(|c| Cluster {
            coord: [
                js::round((c.sum_x / c.count as f64) * 100.0) / 100.0,
                js::round((c.sum_y / c.count as f64) * 100.0) / 100.0,
            ],
            members: c.members,
        })
        .collect()
}

/// Greedy clustering can leave two clusters whose centroids are closer than the threshold.
/// Merges the first such pair until none is left.
fn merge_close_clusters(clusters: &mut Vec<GrowingCluster>, threshold: f64) {
    while let Some((i, j)) = first_close_pair(clusters, threshold) {
        let b = clusters.remove(j);
        let a = &mut clusters[i];
        a.members.extend(b.members);
        a.count += b.count;
        a.sum_x += b.sum_x;
        a.sum_y += b.sum_y;
    }
}

fn first_close_pair(clusters: &[GrowingCluster], threshold: f64) -> Option<(usize, usize)> {
    for i in 0..clusters.len() {
        for j in i + 1..clusters.len() {
            let ([ax, ay], [bx, by]) = (clusters[i].centroid(), clusters[j].centroid());
            let (dx, dy) = (ax - bx, ay - by);
            if (dx * dx + dy * dy).sqrt() <= threshold + CLUSTER_EPSILON {
                return Some((i, j));
            }
        }
    }
    None
}

/// TS `coordKey`: `${zoneID}:${x.toFixed(4)},${y.toFixed(4)}`. Num compares zones the way
/// `String(zoneID)` does (-0 and 0 alike).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct CoordKey(Num, String, String);

fn coord_key(zone_id: f64, x: f64, y: f64) -> CoordKey {
    CoordKey(Num(zone_id), js::to_fixed_4(x), js::to_fixed_4(y))
}

/// The zones in the order a JS object iterates them: array-index keys (integers
/// 0 ..= 2^32 - 2) ascending, then the others in insertion order.
fn in_js_property_order(mut zones: Spawns) -> Spawns {
    let array_index = |zone: f64| (zone.fract() == 0.0 && (0.0..=4_294_967_294.0).contains(&zone)).then_some(zone);
    zones.sort_by(|(a, _), (b, _)| match (array_index(*a), array_index(*b)) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
    zones
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::Confidence;
    use SpawnToken::{Npc, QuestNpc, QuestPoi, Target};

    fn obs(zone_id: f64, x: f64, y: f64, t: f64, token: SpawnToken) -> Observation<SpawnObservation> {
        Observation::new(823, SpawnObservation { zone_id, x, y, token }, Confidence::High, t)
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
        let observations: Vec<_> = points
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| obs(40.0, x, y, i as f64 + 1.0, QuestNpc))
            .collect();
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
        let observations = [
            obs(267.0, 62.52, 19.74, 1.0, Target),
            obs(267.0, 62.57, 19.9, 2.0, Target),
            obs(267.0, 62.59, 19.75, 3.0, Target),
        ];
        assert_eq!(merge_spawns(&observations, false), [(267.0, vec![[62.56, 19.8]])]);
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
        let observations = [obs(40.0, 10.0, 50.0, 1.0, Npc), obs(40.0, 30.0, 60.0, 2.0, Npc)];
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
        assert_eq!(merge_spawns(&observations, false), [(40.0, vec![[10.23, 50.21]])]);
    }

    #[test]
    fn uses_a_larger_grouping_radius_in_capital_cities() {
        let spread = |zone_id| [obs(zone_id, 50.0, 50.0, 1.0, Npc), obs(zone_id, 50.8, 50.0, 2.0, Npc)];
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
    fn spawns_of_no_observations_are_empty() {
        assert_eq!(merge_spawns(&[], false), []);
    }

    #[test]
    fn handles_several_zones_with_several_coordinates_each() {
        let observations = [
            obs(40.0, 30.01, 86.02, 1.0, Npc),
            obs(12.0, 47.46, 62.18, 2.0, Npc),
            obs(40.0, 31.0, 87.0, 3.0, Npc),
            obs(12.0, 48.0, 63.0, 4.0, Npc),
        ];
        assert_eq!(
            merge_spawns(&observations, false),
            [(12.0, vec![[47.46, 62.18], [48.0, 63.0]]), (40.0, vec![[30.01, 86.02], [31.0, 87.0]])]
        );
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
            obs(1519.0, 1.0, 1.0, 1.0, Npc),
            obs(-1.0, 2.0, 2.0, 2.0, Npc),
            obs(12.0, 3.0, 3.0, 3.0, Npc),
            obs(0.5, 4.0, 4.0, 4.0, Npc),
        ];
        let zones: Vec<f64> = merge_spawns(&observations, false).into_iter().map(|(zone, _)| zone).collect();
        assert_eq!(zones, [12.0, 1519.0, -1.0, 0.5]);
    }
}
