// Custom merge functions for object spawns and zoneID fields.
//
// Mirrors the npc/spawnsMerge.ts logic exactly.

import type { Observation } from "../../observation";
import type { SpawnObservationValue, TaggedSpawnObservation } from "./spawns";

export type { TaggedSpawnObservation, SpawnObservationValue } from "./spawns";

const TOKEN_PRIORITY: Record<string, number> = {
  questpoi: 4,
  questnpc: 3,
  npc: 2,
  target: 1,
};

// Max distance (in Questie 0-100 map units) from a cluster's centroid. Covers the
// spread caused by recording the player's position within interaction range.
const SPAWN_CLUSTER_THRESHOLD = 0.5;
const CLUSTER_EPSILON = 1e-9; // Tolerance for floating point comparisons

function tokenPriority(token: string): number {
  return TOKEN_PRIORITY[token] ?? 0;
}

/** Tokens whose coordinates replace all other sources, in priority order. */
const EXCLUSIVE_TOKENS = ["questpoi", "questnpc"];

function findExclusiveToken(observations: TaggedSpawnObservation[]): string | undefined {
  return EXCLUSIVE_TOKENS.find((token) => observations.some((obs) => obs.token === token));
}

/**
 * Clusters nearby spawn coordinates and averages each cluster.
 * A point joins the nearest cluster whose centroid is within threshold, which
 * avoids chaining a long line of points (e.g. a patrol) into one cluster.
 */
function clusterAndAverageCoordinatess(
  coords: Array<[number, number]>,
  threshold: number,
): Array<[number, number]> {
  if (coords.length === 0) return [];
  if (coords.length === 1) {
    // Round singletons too, so every returned coordinate matches the 2-decimal
    // precision of cluster centroids.
    return [[
      Math.round(coords[0][0] * 100) / 100,
      Math.round(coords[0][1] * 100) / 100,
    ]];
  }

  // Sort coords first to ensure consistent clustering order
  coords = [...coords].sort((a, b) => a[0] - b[0] || a[1] - b[1]);

  const clusters: Array<{ count: number; sumX: number; sumY: number }> = [];

  for (const [x, y] of coords) {
    let nearest: { count: number; sumX: number; sumY: number } | undefined;
    let nearestDist = Infinity;
    for (const cluster of clusters) {
      const dx = x - cluster.sumX / cluster.count;
      const dy = y - cluster.sumY / cluster.count;
      const dist = Math.sqrt(dx * dx + dy * dy);
      if (dist < nearestDist) {
        nearestDist = dist;
        nearest = cluster;
      }
    }

    if (nearest && nearestDist <= threshold + CLUSTER_EPSILON) {
      nearest.count += 1;
      nearest.sumX += x;
      nearest.sumY += y;
    } else {
      clusters.push({ count: 1, sumX: x, sumY: y });
    }
  }

  // Return the centroid of each cluster, rounded to 2 decimal places
  return clusters.map((c) => [
    Math.round((c.sumX / c.count) * 100) / 100,
    Math.round((c.sumY / c.count) * 100) / 100,
  ]);
}

export function mergeSpawns(
  observations: Observation<SpawnObservationValue>[],
): Record<number, Array<[number, number]>> {
  const tagged = observations as TaggedSpawnObservation[];

  const exclusiveToken = findExclusiveToken(tagged);

  if (exclusiveToken) {
    // questpoi/questnpc override everything: only keep coordinates of that token
    const byZone = new Map<number, Array<[number, number]>>();
    for (const obs of tagged) {
      if (obs.token !== exclusiveToken) continue;
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
  const bestPerCoord = new Map<string, TaggedSpawnObservation>();

  for (const obs of tagged) {
    const { zoneID, x, y } = obs.value;
    const key = `${zoneID}:${x.toFixed(4)},${y.toFixed(4)}`;
    const existing = bestPerCoord.get(key);
    if (!existing || tokenPriority(obs.token) > tokenPriority(existing.token)) {
      bestPerCoord.set(key, obs);
    }
  }

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

export function mergeZoneID(
  observations: Observation<SpawnObservationValue>[],
): number {
  if (observations.length === 0) return 0;

  const zoneWeights = new Map<number, number>();
  const tagged = observations as TaggedSpawnObservation[];
  const hasQuestpoi = tagged.some((obs) => obs.token === "questpoi");
  for (const obs of tagged) {
    if (hasQuestpoi && obs.token !== "questpoi") continue;
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
