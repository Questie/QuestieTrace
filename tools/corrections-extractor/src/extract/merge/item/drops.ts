// Custom merge for item.npcDrops and item.objectDrops.
//
// Individual observations carry one { npcID: N } or { objectID: N } piece.
// The merge unions them into a number[] per item, deduplicated and sorted.

import type { Observation } from "../../observation";

export function mergeNpcDrops(
  observations: Observation<{ npcID: number }>[],
): number[] {
  const ids = new Set<number>();
  for (const obs of observations) {
    ids.add(obs.value.npcID);
  }
  return [...ids].sort((a, b) => a - b);
}

export function mergeObjectDrops(
  observations: Observation<{ objectID: number }>[],
): number[] {
  const ids = new Set<number>();
  for (const obs of observations) {
    ids.add(obs.value.objectID);
  }
  return [...ids].sort((a, b) => a - b);
}
