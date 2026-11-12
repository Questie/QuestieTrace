// questKeys.startedBy - Questie field 2. Observed in observers/quest/startedBy.ts.
//
// Custom merge: set-union per starter type across all observations.

import type { Observation } from "../../observation";
import type { StartedByStarter } from "../../observers/quest/startedBy";

export interface StartedByValue {
  creatures: number[];
  objects: number[];
  items: number[];
}

export function mergeStartedBy(observations: Observation<StartedByStarter>[]): StartedByValue {
  const creatures = new Set<number>();
  const objects = new Set<number>();
  const items = new Set<number>();

  for (const obs of observations) {
    const s = obs.value;
    if (s.creatureId) creatures.add(s.creatureId);
    if (s.objectId) objects.add(s.objectId);
    if (s.itemId) items.add(s.itemId);
  }

  return {
    creatures: [...creatures].sort((a, b) => a - b),
    objects: [...objects].sort((a, b) => a - b),
    items: [...items].sort((a, b) => a - b),
  };
}
