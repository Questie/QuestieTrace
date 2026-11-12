// questKeys.finishedBy - Questie field 3. Observed in observers/quest/finishedBy.ts.
//
// Custom merge: set-union per finisher type across all observations.

import type { Observation } from "../../observation";
import type { FinishedByFinisher } from "../../observers/quest/finishedBy";

export interface FinishedByValue {
  creatures: number[];
  objects: number[];
}

export function mergeFinishedBy(observations: Observation<FinishedByFinisher>[]): FinishedByValue {
  const creatures = new Set<number>();
  const objects = new Set<number>();

  for (const obs of observations) {
    const f = obs.value;
    if (f.creatureId) creatures.add(f.creatureId);
    if (f.objectId) objects.add(f.objectId);
  }

  return {
    creatures: [...creatures].sort((a, b) => a - b),
    objects: [...objects].sort((a, b) => a - b),
  };
}
