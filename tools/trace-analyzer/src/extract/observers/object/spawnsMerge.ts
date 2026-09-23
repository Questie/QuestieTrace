// Custom merge functions for object spawns and zoneID fields.
//
// Mirrors the npc/spawnsMerge.ts logic exactly.

import type { Observation } from "../../observation";
import type { SpawnObservationValue, TaggedSpawnObservation } from "./spawns";

export type { TaggedSpawnObservation, SpawnObservationValue } from "./spawns";

const TOKEN_PRIORITY: Record<string, number> = {
  questnpc: 3,
  npc: 2,
  target: 1,
};

function tokenPriority(token: string): number {
  return TOKEN_PRIORITY[token] ?? 0;
}

export function mergeSpawns(
  observations: Observation<SpawnObservationValue>[],
): Record<number, Array<[number, number]>> {
  const bestPerCoord = new Map<string, TaggedSpawnObservation>();

  for (const obs of observations as TaggedSpawnObservation[]) {
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
    coords.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    result[zoneID] = coords;
  }

  return result;
}

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
