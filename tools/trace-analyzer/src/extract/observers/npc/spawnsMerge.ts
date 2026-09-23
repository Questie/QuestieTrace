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

function tokenPriority(token: string): number {
  return TOKEN_PRIORITY[token] ?? 0;
}

/**
 * Merges individual spawn observations into the Questie spawns table shape:
 *   {[zoneID]: {{x,y}, {x,y}, ...}, ...}
 *
 * When the same coordinate is observed via different tokens (e.g. once via
 * questnpc and once via target), the questnpc observation wins because the
 * NPC is guaranteed adjacent to the player during quest interactions.
 */
export function mergeSpawns(
  observations: Observation<SpawnObservationValue>[],
): Record<number, Array<[number, number]>> {
  // For each zone+coordinate pair, keep only the observation with the highest
  // token priority (questnpc > npc > target).
  const bestPerCoord = new Map<string, TaggedSpawnObservation>();

  for (const obs of observations as TaggedSpawnObservation[]) {
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
    coords.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    result[zoneID] = coords;
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
