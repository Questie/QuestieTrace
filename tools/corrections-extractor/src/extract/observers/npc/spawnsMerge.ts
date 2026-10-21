// Custom merge functions for npc spawns and zoneID fields.
//
// spawns: accumulate per-zone coordinate sets from individual observations.
// zoneID: pick the most frequently observed zone from spawn observations.

import type { Observation } from "../../observation";
import type { SpawnObservationValue, TaggedSpawnObservation } from "./spawns";

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
 * Greedy clustering can leave two clusters whose centroids are closer than the
 * threshold. Merges such pairs until all centroids are further apart.
 */
function mergeCloseClusters<C extends { members: Array<[number, number]>; count: number; sumX: number; sumY: number }>(
  clusters: C[],
  threshold: number,
): void {
  let merged = true;
  while (merged) {
    merged = false;
    for (let i = 0; i < clusters.length && !merged; i++) {
      for (let j = i + 1; j < clusters.length && !merged; j++) {
        const a = clusters[i];
        const b = clusters[j];
        const dx = a.sumX / a.count - b.sumX / b.count;
        const dy = a.sumY / a.count - b.sumY / b.count;
        if (Math.sqrt(dx * dx + dy * dy) <= threshold + CLUSTER_EPSILON) {
          a.members.push(...b.members);
          a.count += b.count;
          a.sumX += b.sumX;
          a.sumY += b.sumY;
          clusters.splice(j, 1);
          merged = true;
        }
      }
    }
  }
}

/**
 * Clusters nearby spawn coordinates and averages each cluster.
 * A point joins the nearest cluster whose centroid is within threshold, which
 * avoids chaining a long line of points (e.g. a patrol) into one cluster.
 * Each centroid is returned with the number of points in its cluster.
 */
function clusterAndAverageCoordinatess(
  coords: Array<[number, number]>,
  threshold: number,
): Array<{ coord: [number, number]; members: Array<[number, number]> }> {
  // Sort coords first to ensure consistent clustering order
  coords = [...coords].sort((a, b) => a[0] - b[0] || a[1] - b[1]);

  const clusters: Array<{ members: Array<[number, number]>; count: number; sumX: number; sumY: number }> = [];

  for (const [x, y] of coords) {
    let nearest: (typeof clusters)[number] | undefined;
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
      nearest.members.push([x, y]);
      nearest.count += 1;
      nearest.sumX += x;
      nearest.sumY += y;
    } else {
      clusters.push({ members: [[x, y]], count: 1, sumX: x, sumY: y });
    }
  }

  mergeCloseClusters(clusters, threshold);

  // Return the centroid of each cluster, rounded to 2 decimal places
  return clusters.map((c) => ({
    coord: [
      Math.round((c.sumX / c.count) * 100) / 100,
      Math.round((c.sumY / c.count) * 100) / 100,
    ],
    members: c.members,
  }));
}

function coordKey(zoneID: number, x: number, y: number): string {
  return `${zoneID}:${x.toFixed(4)},${y.toFixed(4)}`;
}

/**
 * Clusters each zone's distinct coordinates. With `singleSpawnPerZone`, only
 * the centroid of the cluster with the most observations is kept per zone.
 */
function clusterByZone(
  byZone: Map<number, Array<[number, number]>>,
  observationCounts: Map<string, number>,
  singleSpawnPerZone: boolean,
): Record<number, Array<[number, number]>> {
  const result: Record<number, Array<[number, number]>> = {};
  for (const [zoneID, coords] of byZone) {
    let clusters = clusterAndAverageCoordinatess(coords, SPAWN_CLUSTER_THRESHOLD);
    if (singleSpawnPerZone) {
      const weight = (members: Array<[number, number]>) =>
        members.reduce((sum, [x, y]) => sum + (observationCounts.get(coordKey(zoneID, x, y)) ?? 1), 0);
      clusters = [clusters.reduce((best, c) => (weight(c.members) > weight(best.members) ? c : best))];
    }
    const spawns = clusters.map((c) => c.coord);
    spawns.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    result[zoneID] = spawns;
  }
  return result;
}

/**
 * Merges individual spawn observations into the Questie spawns table shape:
 *   {[zoneID]: {{x,y}, {x,y}, ...}, ...}
 *
 * When the same NPC has observations via different tokens at different
 * coordinates, questpoi observations (quest turn-in POIs from
 * C_QuestLog.GetQuestsOnMap) overrule ALL others, followed by questnpc.
 * Only coordinates of the highest such token are kept when present.
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
  singleSpawnPerZone = false,
): Record<number, Array<[number, number]>> {
  const tagged = observations as TaggedSpawnObservation[];

  const exclusiveToken = findExclusiveToken(tagged);
  const observationCounts = new Map<string, number>();
  for (const obs of tagged) {
    if (exclusiveToken && obs.token !== exclusiveToken) continue;
    const key = coordKey(obs.value.zoneID, obs.value.x, obs.value.y);
    observationCounts.set(key, (observationCounts.get(key) ?? 0) + 1);
  }

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

    return clusterByZone(byZone, observationCounts, singleSpawnPerZone);
  }

  // No questnpc observations: fall back to per-coordinate priority merging
  // For each zone+coordinate pair, keep only the observation with the highest
  // token priority (questnpc > npc > target).
  const bestPerCoord = new Map<string, TaggedSpawnObservation>();

  for (const obs of tagged) {
    const { zoneID, x, y } = obs.value;
    const key = coordKey(zoneID, x, y);
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

  return clusterByZone(byZone, observationCounts, singleSpawnPerZone);
}

/**
 * Derives zoneID from spawn observations: the most frequently observed zone.
 * When counting, questnpc observations are weighted higher (count as 3) since
 * they represent adjacent NPC interactions, vs regular encounters (count as 1).
 * When questpoi observations exist, only those are counted.
 * Returns 0 (schema default) when no observations exist.
 */
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
