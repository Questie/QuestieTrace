//! Spawn observations, shared by NPC and object spawns and by quest turn-in POIs (which join
//! their finisher's spawns at finalize), and the clustering both spawn merges place them with.
//!
//! The npc and object merges were one merge in TS; they are two copies there now, which differ
//! in which observations they use and where they place a cluster. What they still share is here.

use crate::aggregate::Num;
use crate::js;
use crate::observation::Observation;
use indexmap::map::Entry;
use indexmap::IndexMap;
use std::cmp::Ordering;

/// Where a spawn position came from. Merges rank positions by it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SpawnToken {
    Target,
    Npc,
    QuestNpc,
    /// A quest turn-in POI attached to the quest's single finisher.
    QuestPoi,
    /// A loot of an object: the player stood next to it. Only objects are looted.
    Loot,
}

impl SpawnToken {
    /// The unit token an encounter was seen through.
    pub fn from_unit_token(token: &str) -> Option<SpawnToken> {
        match token {
            "target" => Some(SpawnToken::Target),
            "npc" => Some(SpawnToken::Npc),
            "questnpc" => Some(SpawnToken::QuestNpc),
            "questpoi" => Some(SpawnToken::QuestPoi),
            _ => None,
        }
    }

    /// How close to the entity the player was: the merges rank positions by it. (The npc merge's
    /// table has no loot in TS; npc spawns never carry it.)
    pub fn priority(self) -> u32 {
        match self {
            SpawnToken::QuestPoi => 4,
            SpawnToken::QuestNpc | SpawnToken::Loot => 3,
            SpawnToken::Npc => 2,
            SpawnToken::Target => 1,
        }
    }
}

/// One spawn point seen in a zone (TS `SpawnObservationValue` plus the tagged `token`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpawnObservation {
    /// AreaTable areaID.
    pub zone_id: f64,
    /// Questie map coordinates, 0-100.
    pub x: f64,
    pub y: f64,
    pub token: SpawnToken,
}

/// A spawns table `{[zoneID]: [[x, y], ...]}`: per zone (AreaTable areaID) its spawn points in
/// Questie 0-100 map coordinates. Zones are in JS property order (integer ids ascending), the
/// order `Object.entries` gives the Lua writer.
pub type Spawns = Vec<(f64, Vec<[f64; 2]>)>;

/// The zone the most observations were made in: questnpc and loot count 3, npc 2 and target 1. When
/// questpoi observations exist only they count. 0 (the schema default) without observations.
/// The npc and object merges' mergeZoneID are identical.
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
        *zone_weights.entry(Num(obs.value.zone_id + 0.0)).or_insert(0) += obs.value.token.priority();
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

/// The first of `tokens` (in priority order) any observation has: its coordinates replace all
/// other sources.
pub(crate) fn find_exclusive_token(observations: &[Observation<SpawnObservation>], tokens: &[SpawnToken]) -> Option<SpawnToken> {
    tokens.iter().copied().find(|&token| observations.iter().any(|obs| obs.value.token == token))
}

/// Each zone's distinct coordinates, zones in the order first seen. Coordinates of an exclusive
/// token: identical ones are kept once.
pub(crate) fn distinct_coordinates_by_zone<'o>(spawns: impl Iterator<Item = &'o SpawnObservation>) -> IndexMap<Num, Vec<[f64; 2]>> {
    let mut by_zone: IndexMap<Num, Vec<[f64; 2]>> = IndexMap::new();
    for &SpawnObservation { zone_id, x, y, .. } in spawns {
        // A JS Map stores a -0 key as +0.
        let coords = by_zone.entry(Num(zone_id + 0.0)).or_default();
        if !coords.iter().any(|c| c[0] == x && c[1] == y) {
            coords.push([x, y]);
        }
    }
    by_zone
}

/// Each zone's coordinates, zones in the order first seen: per coordinate key the observation of
/// the highest token priority. A replaced observation keeps the position of the first one, as
/// Map.set does.
pub(crate) fn best_coordinates_by_zone<'o>(spawns: impl Iterator<Item = &'o SpawnObservation>) -> IndexMap<Num, Vec<[f64; 2]>> {
    let mut best_per_coord: IndexMap<CoordKey, &SpawnObservation> = IndexMap::new();
    for spawn in spawns {
        match best_per_coord.entry(coord_key(spawn.zone_id, spawn.x, spawn.y)) {
            Entry::Occupied(mut best) => {
                if spawn.token.priority() > best.get().token.priority() {
                    best.insert(spawn);
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(spawn);
            }
        }
    }
    let mut by_zone: IndexMap<Num, Vec<[f64; 2]>> = IndexMap::new();
    for spawn in best_per_coord.values() {
        by_zone.entry(Num(spawn.zone_id + 0.0)).or_default().push([spawn.x, spawn.y]);
    }
    by_zone
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

/// The clustering radius in a zone.
pub(crate) fn cluster_threshold(zone_id: f64) -> f64 {
    if CAPITAL_CITY_ZONE_IDS.contains(&zone_id) {
        CAPITAL_CITY_CLUSTER_THRESHOLD
    } else {
        SPAWN_CLUSTER_THRESHOLD
    }
}

/// A cluster of nearby coordinates. The sums are kept as TS accumulates them: a centroid
/// recomputed from the members could differ in the last bit.
pub(crate) struct CoordCluster {
    pub members: Vec<[f64; 2]>,
    count: usize,
    sum_x: f64,
    sum_y: f64,
}

impl CoordCluster {
    pub fn centroid(&self) -> [f64; 2] {
        [self.sum_x / self.count as f64, self.sum_y / self.count as f64]
    }
}

/// Clusters nearby coordinates. A point joins the nearest cluster whose centroid is within
/// `threshold`, which avoids chaining a line of points (a patrol) into one cluster.
pub(crate) fn cluster_coordinates(coords: &[[f64; 2]], threshold: f64) -> Vec<CoordCluster> {
    // Sorted first for a consistent clustering order.
    let mut coords = coords.to_vec();
    coords.sort_by(compare_coords);

    let mut clusters: Vec<CoordCluster> = Vec::new();
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
            _ => clusters.push(CoordCluster {
                members: vec![[x, y]],
                count: 1,
                sum_x: x,
                sum_y: y,
            }),
        }
    }

    merge_close_clusters(&mut clusters, threshold);
    clusters
}

/// Greedy clustering can leave two clusters whose centroids are closer than the threshold.
/// Merges the first such pair until none is left.
fn merge_close_clusters(clusters: &mut Vec<CoordCluster>, threshold: f64) {
    while let Some((i, j)) = first_close_pair(clusters, threshold) {
        let b = clusters.remove(j);
        let a = &mut clusters[i];
        a.members.extend(b.members);
        a.count += b.count;
        a.sum_x += b.sum_x;
        a.sum_y += b.sum_y;
    }
}

fn first_close_pair(clusters: &[CoordCluster], threshold: f64) -> Option<(usize, usize)> {
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

/// `Math.round(value * 100) / 100`: spawn coordinates have 2 decimals.
pub(crate) fn round_2_decimals(value: f64) -> f64 {
    js::round(value * 100.0) / 100.0
}

/// `(a, b) => a - b`, read the way JS sort reads it (NaN is equal).
pub(crate) fn compare_numbers(a: f64, b: f64) -> Ordering {
    let difference = a - b;
    if difference < 0.0 {
        Ordering::Less
    } else if difference > 0.0 {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// `(a, b) => a[0] - b[0] || a[1] - b[1]`, read the way JS sort reads it (NaN is equal).
pub(crate) fn compare_coords(a: &[f64; 2], b: &[f64; 2]) -> Ordering {
    let dx = a[0] - b[0];
    if dx == 0.0 || dx.is_nan() {
        compare_numbers(a[1], b[1])
    } else {
        compare_numbers(a[0], b[0])
    }
}

/// TS `coordKey`: `${zoneID}:${x.toFixed(4)},${y.toFixed(4)}`. Num compares zones the way
/// `String(zoneID)` does (-0 and 0 alike).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct CoordKey(Num, String, String);

pub(crate) fn coord_key(zone_id: f64, x: f64, y: f64) -> CoordKey {
    CoordKey(Num(zone_id), js::to_fixed_4(x), js::to_fixed_4(y))
}

/// The zones in the order a JS object iterates them: array-index keys (integers
/// 0 ..= 2^32 - 2) ascending, then the others in insertion order.
pub(crate) fn in_js_property_order(mut zones: Spawns) -> Spawns {
    let array_index = |zone: f64| (zone.fract() == 0.0 && (0.0..=4_294_967_294.0).contains(&zone)).then_some(zone);
    zones.sort_by(|(a, _), (b, _)| match (array_index(*a), array_index(*b)) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
    zones
}
