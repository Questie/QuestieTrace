// Custom merge functions for npc spawns and zoneID fields.
//
// spawns: accumulate per-zone coordinate sets from individual observations.
// zoneID: pick the most frequently observed zone from spawn observations.

import type { Observation } from "../../observation";
import type { SpawnObservationValue, TaggedSpawnObservation } from "./spawns";

const TOKEN_PRIORITY: Record<string, number> = {
  questnpc: 3,
  npc: 2,
  target: 1,
};

const SPAWN_CLUSTER_THRESHOLD = 0.05; // 5% of map coordinate range
const CLUSTER_EPSILON = 1e-9; // Tolerance for floating point comparisons

function tokenPriority(token: string): number {
  return TOKEN_PRIORITY[token] ?? 0;
}

/**
 * Clusters nearby spawn coordinates and averages each cluster.
 * Two points are in the same cluster if their Euclidean distance is <= threshold.
 */
function clusterAndAverageCoordinatess(
  coords: Array<[number, number]>,
  threshold: number,
): Array<[number, number]> {
  if (coords.length === 0) return [];
  if (coords.length === 1) return coords;

  // Sort coords first to ensure consistent clustering order
  coords = [...coords].sort((a, b) => a[0] - b[0] || a[1] - b[1]);

  // Simple greedy clustering: for each point, find if it belongs to an existing cluster
  const clusters: Array<{ points: Array<[number, number]>; sumX: number; sumY: number }> = [];

  for (const [x, y] of coords) {
    // Find every existing cluster containing a point within threshold of (x, y)
    const matched: Array<{ points: Array<[number, number]>; sumX: number; sumY: number }> = [];
    for (const cluster of clusters) {
      // Check if this point is within threshold of any point in the cluster
      for (const [cx, cy] of cluster.points) {
        const dx = x - cx;
        const dy = y - cy;
        const dist = Math.sqrt(dx * dx + dy * dy);
        if (dist <= threshold + CLUSTER_EPSILON) {
          matched.push(cluster);
          break;
        }
      }
    }

    if (matched.length === 0) {
      // Start a new cluster
      clusters.push({
        points: [[x, y]],
        sumX: x,
        sumY: y,
      });
    } else {
      // Merge all matched clusters into the first one, then add the new point
      const [primary, ...others] = matched;
      for (const cluster of others) {
        primary.points.push(...cluster.points);
        primary.sumX += cluster.sumX;
        primary.sumY += cluster.sumY;
        clusters.splice(clusters.indexOf(cluster), 1);
      }
      primary.points.push([x, y]);
      primary.sumX += x;
      primary.sumY += y;
    }
  }

  // Return the centroid of each cluster, rounded to 2 decimal places
  return clusters.map((c) => [
    Math.round((c.sumX / c.points.length) * 100) / 100,
    Math.round((c.sumY / c.points.length) * 100) / 100,
  ]);
}

/**
 * Merges individual spawn observations into the Questie spawns table shape:
 *   {[zoneID]: {{x,y}, {x,y}, ...}, ...}
 *
 * When the same NPC has observations via different tokens at different
 * coordinates, questnpc observations overrule ALL others. Only questnpc
 * coordinates are kept when questnpc observations exist for that NPC.
 *
 * This is because questnpc fires during quest interactions when the player
 * must be adjacent to the NPC, giving us the true spawn location. Target/npc
 * tokens can fire from a distance and give inaccurate positions.
 *
 * When no questnpc observations exist, falls back to keeping the best
 * coordinate per (zoneID, x, y) based on token priority.
 */
export function mergeSpawns(
  observations: Observation<SpawnObservationValue>[],
): Record<number, Array<[number, number]>> {
  const tagged = observations as TaggedSpawnObservation[];

  // Check if any questnpc observations exist for this NPC
  const hasQuestnpc = tagged.some((obs) => obs.token === "questnpc");

  if (hasQuestnpc) {
    // questnpc overrides everything: only keep questnpc coordinates
    const byZone = new Map<number, Array<[number, number]>>();
    for (const obs of tagged) {
      if (obs.token !== "questnpc") continue;
      const { zoneID, x, y } = obs.value;
      const coords = byZone.get(zoneID);
      if (coords) {
        // Deduplicate identical coordinates within a zone
        if (!coords.some((c) => c[0] === x && c[1] === y)) {
          coords.push([x, y]);
        }
      } else {
        byZone.set(zoneID, [[x, y]]);
      }
    }

    const result: Record<number, Array<[number, number]>> = {};
    for (const [zoneID, coords] of byZone) {
      // Cluster nearby coordinates and average each cluster
      const clustered = clusterAndAverageCoordinatess(coords, SPAWN_CLUSTER_THRESHOLD);
      clustered.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
      result[zoneID] = clustered;
    }
    return result;
  }

  // No questnpc observations: fall back to per-coordinate priority merging
  // For each zone+coordinate pair, keep only the observation with the highest
  // token priority (questnpc > npc > target).
  const bestPerCoord = new Map<string, TaggedSpawnObservation>();

  for (const obs of tagged) {
    const { zoneID, x, y } = obs.value;
    const key = `${zoneID}:${x.toFixed(4)},${y.toFixed(4)}`;
    const existing = bestPerCoord.get(key);
    if (!existing || tokenPriority(obs.token) > tokenPriority(existing.token)) {
      bestPerCoord.set(key, obs);
    }
  }

  // Group by zone
  const byZone = new Map<number, Array<[number, number]>>();
  for (const obs of bestPerCoord.values()) {
    const { zoneID, x, y } = obs.value;
    const coords = byZone.get(zoneID);
    if (coords) {
      coords.push([x, y]);
    } else {
      byZone.set(zoneID, [[x, y]]);
    }
  }

  const result: Record<number, Array<[number, number]>> = {};
  for (const [zoneID, coords] of byZone) {
    // Cluster nearby coordinates and average each cluster
    const clustered = clusterAndAverageCoordinatess(coords, SPAWN_CLUSTER_THRESHOLD);
    clustered.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    result[zoneID] = clustered;
  }

  return result;
}

/**
 * Derives zoneID from spawn observations: the most frequently observed zone.
 * When counting, questnpc observations are weighted higher (count as 3) since
 * they represent adjacent NPC interactions, vs regular encounters (count as 1).
 * Returns 0 (schema default) when no observations exist.
 */
export function mergeZoneID(
  observations: Observation<SpawnObservationValue>[],
): number {
  if (observations.length === 0) return 0;

  const zoneWeights = new Map<number, number>();
  for (const obs of observations as TaggedSpawnObservation[]) {
    const { zoneID } = obs.value;
    const weight = tokenPriority(obs.token);
    zoneWeights.set(zoneID, (zoneWeights.get(zoneID) ?? 0) + weight);
  }

  let bestZone = 0;
  let bestWeight = 0;
  for (const [zoneID, weight] of zoneWeights) {
    if (weight > bestWeight) {
      bestWeight = weight;
      bestZone = zoneID;
    }
  }

  return bestZone;
}
